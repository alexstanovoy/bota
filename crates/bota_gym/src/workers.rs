//! The simulation workers and the caller's per-slot logic they run.
//!
//! A run's games are [`Episode`]s: the caller's code for one game of one slot (its match and
//! whatever the seats' clients make of their streams: trackers, observation encoding, rewards,
//! scripted bots). [`Workers`] run them: `threads` threads take jobs from one bounded FIFO
//! queue, each job owning one boxed episode, so which worker runs it never changes its result.
//!
//! Drivers ([`VecEnv`](crate::VecEnv), [`Tournament`](crate::Tournament)) submit their whole
//! batch, then wait for all of it; replies come back to the submitting driver's own channel and
//! are put in position order. Any number of drivers, each on its own thread, may share one
//! `Workers` (all workers serve every driver) or own one each (say, pinned to a cache domain):
//! results are the same either way.
//!
//! **Fairness and backpressure.** The queue is first in, first out and holds at most
//! `queue` jobs. A driver submitting into a full queue waits for a worker to free a place; a
//! worker never waits (a driver's reply channel has room for its whole batch). So every job
//! submitted finishes, no driver can starve another, and a job waits behind at most the
//! `queue` jobs already queued plus the ones being run.
//!
//! **Caller code on workers** must be bounded: an episode's `start` and `step` run on a
//! shared worker, so a step that blocks (on I/O, a lock held elsewhere, another driver) stalls
//! that worker for everyone. A panic in caller code is caught on the worker and reported as
//! [`GymError::Panicked`] for that position, after its payload is dropped; the episode is lost
//! and its driver is spent, while the workers and other drivers carry on.
//!
//! **Destructors that panic.** A payload's destructor is caller code too. When it panics, the
//! payload of that panic is dropped as well. When that one's destructor panics again, the caller's
//! destructors have proven unsafe to run: the pool keeps that payload (leaks it) and turns
//! *terminal*. A terminal pool runs nothing more and drops nothing more of the caller's: every
//! job still queued, or enqueued later by a submitter that was blocked on a full queue, is
//! answered with [`GymError::Undisposed`] and leaked whole (its episode, action and plan are
//! never dropped). A submission after that is refused with the same error on the submitting
//! thread, which still owns (and drops) what it tried to submit. What a pool leaks
//! is therefore at most one payload for each worker that was running a job when the pool turned
//! terminal (at most `threads`), plus the jobs that were queued or blocked in submission then
//! (at most the queue's capacity plus the submitters waiting on it, each one accepted job of a
//! live driver), each leaked whole and none executed. Every accepted job is answered exactly
//! once and no driver hangs. Leaked jobs may hold handles to the pool itself, so a terminal pool
//! does not wait for every handle to go: once nothing is queued and no submission that saw it
//! healthy is still sending, its workers leave, and the scope they run in can end.
//!
//! Not guaranteed: anything about payload sizes (they are the caller's); anything about the
//! caller's destructors that run on the caller's own threads (a refused submission, the episodes
//! and actions a spent driver still holds when it is dropped), whose panics are the caller's; and
//! a panic raised while another panic unwinds (a destructor that panics during unwinding), which
//! Rust turns into an abort of the process.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{GameId, GymError};

/// Upper bound on worker threads.
pub const MAX_THREADS: usize = 256;
/// Upper bound on a worker queue.
pub const MAX_QUEUE: usize = 4096;

/// The caller's code for one game of one slot, run on the [`Workers`].
///
/// It must be `Send` (it moves between the driver and the workers) and every method must
/// return in bounded time. Everything it reads besides its own state comes from `Shared`,
/// which must be observably immutable (a cache is fine only if what it returns never depends
/// on what was asked before): that is what makes a run independent of worker count and timing.
pub trait Episode: Sized + Send {
    /// Run-wide state every worker reads: rules, catalogues, scripted bots' settings.
    type Shared: Sync;
    /// What starts one game besides its id.
    type Plan: Send;
    /// What the caller's policy hands one episode per step.
    type Action: Send;
    /// What a finished game hands back.
    type Finished: Send;
    type Error: Send + From<GymError>;

    /// Builds the game `game` of `plan`, ready for its first action. `self.game()` must then
    /// return `game`.
    fn start(shared: &Self::Shared, game: GameId, plan: Self::Plan) -> Result<Self, Self::Error>;

    /// Applies one action and advances the game; returns what the game left once it is over.
    /// A finished episode is never stepped again.
    fn step(
        &mut self,
        shared: &Self::Shared,
        action: Self::Action,
    ) -> Result<Option<Self::Finished>, Self::Error>;

    /// Which game this is.
    fn game(&self) -> GameId;
}

/// How many workers, how long their queue, and where they run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerConfig {
    /// `1..=MAX_THREADS` threads.
    pub threads: usize,
    /// Jobs the queue holds, `1..=MAX_QUEUE`; drivers sharing these workers wait beyond it.
    pub queue: usize,
    /// CPUs every worker is pinned to (see [`topology`](crate::topology)); none lets the OS
    /// place them. Pinning never changes results.
    pub cpus: Option<Vec<usize>>,
    /// Thread name prefix; workers are `{name}-{index}`.
    pub name: String,
}

pub(crate) type Build<'scope, E> =
    Box<dyn FnOnce(&<E as Episode>::Shared) -> Result<E, <E as Episode>::Error> + Send + 'scope>;

pub(crate) enum Job<'scope, E: Episode> {
    Start {
        game: GameId,
        plan: E::Plan,
    },
    Build {
        slot: usize,
        build: Build<'scope, E>,
    },
    Step {
        episode: Box<E>,
        action: E::Action,
        /// The plan of the slot's next game, which starts in this job when this one ends.
        next: Option<E::Plan>,
    },
}

/// Why a job gave back no episode.
pub(crate) enum Failure<Error> {
    /// The caller's code returned an error.
    Error(Error),
    /// The caller's code panicked; the driver turns this into its error, on its own thread.
    Panicked,
    /// The pool turned terminal before this job ran; it never ran.
    Undisposed,
}

/// One job's result: the episode (its next game's, when a game ended and a plan was given) and
/// what a finished game left.
pub(crate) type Done<E> =
    Result<(Box<E>, Option<<E as Episode>::Finished>), Failure<<E as Episode>::Error>>;
pub(crate) type Reply<E> = (usize, Done<E>);

struct Envelope<'scope, E: Episode> {
    position: usize,
    job: Job<'scope, E>,
    reply: SyncSender<Reply<E>>,
}

/// A handle to shared simulation workers; they leave once every handle is dropped, or once the
/// pool turned terminal and what was submitted to it is answered.
pub struct Workers<'scope, E: Episode> {
    jobs: SyncSender<Envelope<'scope, E>>,
    state: Arc<State>,
}

/// What the handles and the workers of one pool share.
#[derive(Default)]
struct State {
    /// Set once the workers kept a panic payload they could not drop: see the module docs.
    terminal: AtomicBool,
    /// Submissions between their check of `terminal` and the end of their send.
    sending: AtomicUsize,
}

/// How long an idle worker waits for a job before it looks whether the pool turned terminal.
/// Leaked jobs may hold handles that keep the queue open, so a terminal pool cannot wait for
/// every handle to go.
const POLL: Duration = Duration::from_millis(20);

impl<E: Episode> Clone for Workers<'_, E> {
    fn clone(&self) -> Self {
        Self {
            jobs: self.jobs.clone(),
            state: Arc::clone(&self.state),
        }
    }
}

impl<'scope, E: Episode + 'scope> Workers<'scope, E> {
    /// Spawns the workers inside `scope`; they read `shared` for as long as they run.
    pub fn spawn<'env>(
        scope: &'scope std::thread::Scope<'scope, 'env>,
        config: WorkerConfig,
        shared: &'scope E::Shared,
    ) -> Result<Self, GymError> {
        if !(1..=MAX_THREADS).contains(&config.threads) {
            return Err(GymError::Config("worker threads must be 1..=MAX_THREADS"));
        }
        if !(1..=MAX_QUEUE).contains(&config.queue) {
            return Err(GymError::Config("worker queue must be 1..=MAX_QUEUE"));
        }
        let (jobs, receiver) = sync_channel::<Envelope<'scope, E>>(config.queue);
        let receiver = Arc::new(Mutex::new(receiver));
        let state = Arc::new(State::default());
        for index in 0..config.threads {
            let receiver = Arc::clone(&receiver);
            let state = Arc::clone(&state);
            let cpus = config.cpus.clone();
            std::thread::Builder::new()
                .name(format!("{}-{index}", config.name))
                .spawn_scoped(scope, move || {
                    // An unpinned worker still simulates correctly, only placed by the OS.
                    if let Some(cpus) = cpus
                        && let Err(error) = crate::topology::pin_current_thread(&cpus)
                    {
                        eprintln!("level=WARN event=pin_failed worker={index} {error}");
                    }
                    work(&receiver, shared, &state);
                })
                .map_err(|_| GymError::Config("a worker thread could not be spawned"))?;
        }
        Ok(Self { jobs, state })
    }

    pub(crate) fn submit(
        &self,
        position: usize,
        job: Job<'scope, E>,
        reply: &SyncSender<Reply<E>>,
    ) -> Result<(), GymError> {
        // Counted before the check: a terminal pool's workers stay until no submission that
        // passed the check is still sending, so every job sent is answered.
        self.state.sending.fetch_add(1, Ordering::SeqCst);
        let sent = if self.state.terminal.load(Ordering::SeqCst) {
            // Refused on the submitting thread, which keeps (and drops) what it submitted.
            Err(GymError::Undisposed)
        } else {
            let envelope = Envelope {
                position,
                job,
                reply: reply.clone(),
            };
            // Waits while the queue is full: the backpressure every driver shares.
            self.jobs.send(envelope).map_err(|_| GymError::Stopped)
        };
        self.state.sending.fetch_sub(1, Ordering::SeqCst);
        sent
    }
}

fn work<E: Episode>(
    receiver: &Mutex<Receiver<Envelope<'_, E>>>,
    shared: &E::Shared,
    state: &State,
) {
    loop {
        let envelope = match receiver.lock() {
            Ok(receiver) => receiver.recv_timeout(POLL),
            Err(_) => return,
        };
        match envelope {
            Ok(envelope) => serve(envelope, shared, &state.terminal),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if state.terminal.load(Ordering::SeqCst) {
            return close(receiver, state);
        }
    }
}

/// Runs one job, or answers it unrun once the pool is terminal.
fn serve<E: Episode>(envelope: Envelope<'_, E>, shared: &E::Shared, terminal: &AtomicBool) {
    let Envelope {
        position,
        job,
        reply,
    } = envelope;
    let mut answer = Answer {
        position,
        reply: Some(reply),
    };
    if terminal.load(Ordering::SeqCst) {
        // The caller's destructors proved unsafe to run: answered, never run, never dropped.
        answer.send(Err(Failure::Undisposed));
        std::mem::forget(job);
        return;
    }
    // Nothing the caller's code raises escapes the worker: not a panic of the job, not a panic
    // of dropping what a job left, not one of dropping the panic's payload.
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| answer.send(run(job, shared)))) {
        dispose(payload, terminal);
    }
    // An answer the job never sent goes now, as a panic.
    drop(answer);
}

/// A terminal pool's last duty: answer every job still queued or still being sent, then leave
/// without waiting for the handles that leaked jobs may hold. A submission that starts after
/// this sees the pool terminal and is refused on its own thread.
fn close<E: Episode>(receiver: &Mutex<Receiver<Envelope<'_, E>>>, state: &State) {
    let Ok(receiver) = receiver.lock() else {
        return;
    };
    loop {
        match receiver.recv_timeout(POLL) {
            Ok(envelope) => serve_terminal(envelope),
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                if state.sending.load(Ordering::SeqCst) == 0 {
                    // Whatever the last sender sent before it finished is queued by now.
                    while let Ok(envelope) = receiver.try_recv() {
                        serve_terminal(envelope);
                    }
                    return;
                }
            }
        }
    }
}

fn serve_terminal<E: Episode>(envelope: Envelope<'_, E>) {
    let _ = envelope
        .reply
        .send((envelope.position, Err(Failure::Undisposed)));
    std::mem::forget(envelope.job);
}

/// Drops a caught panic's payload, before its driver hears of the panic. Should its destructor
/// panic, the payload that panic raised is dropped as well; should that one's destructor panic
/// too, the third payload is kept (leaked) and the pool turns terminal (see the module docs).
fn dispose(payload: Box<dyn Any + Send>, terminal: &AtomicBool) {
    if let Err(second) = catch_unwind(AssertUnwindSafe(|| drop(payload)))
        && let Err(third) = catch_unwind(AssertUnwindSafe(|| drop(second)))
    {
        std::mem::forget(third);
        terminal.store(true, Ordering::SeqCst);
    }
}

/// The reply a job owes its driver, sent exactly once: with the job's result, or as a panic
/// when the job never got to send it.
struct Answer<E: Episode> {
    position: usize,
    reply: Option<SyncSender<Reply<E>>>,
}

impl<E: Episode> Answer<E> {
    fn send(&mut self, done: Done<E>) {
        if let Some(reply) = self.reply.take() {
            // A driver that failed no longer reads its replies; what it would have read is
            // dropped here, inside the worker's catch.
            let _ = reply.send((self.position, done));
        }
    }
}

impl<E: Episode> Drop for Answer<E> {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send((self.position, Err(Failure::Panicked)));
        }
    }
}

fn run<E: Episode>(job: Job<'_, E>, shared: &E::Shared) -> Done<E> {
    match job {
        Job::Start { game, plan } => start(shared, game, plan).map(|episode| (episode, None)),
        Job::Build { slot, build } => {
            let episode = build(shared).map_err(Failure::Error)?;
            if episode.game().slot != slot {
                return Err(Failure::Error(GymError::GameMismatch.into()));
            }
            Ok((Box::new(episode), None))
        }
        Job::Step {
            mut episode,
            action,
            next,
        } => {
            let finished = episode.step(shared, action).map_err(Failure::Error)?;
            match (&finished, next) {
                (Some(_), Some(plan)) => {
                    let game = episode
                        .game()
                        .next()
                        .map_err(|error| Failure::Error(error.into()))?;
                    start(shared, game, plan).map(|next| (next, finished))
                }
                _ => Ok((episode, finished)),
            }
        }
    }
}

fn start<E: Episode>(
    shared: &E::Shared,
    game: GameId,
    plan: E::Plan,
) -> Result<Box<E>, Failure<E::Error>> {
    let episode = E::start(shared, game, plan).map_err(Failure::Error)?;
    if episode.game() != game {
        return Err(Failure::Error(GymError::GameMismatch.into()));
    }
    Ok(Box::new(episode))
}

/// A driver's side of one round: its reply channel, the actions it hands out and the slots
/// replies land in.
pub(crate) struct Round<E: Episode> {
    pub(crate) reply: SyncSender<Reply<E>>,
    replies: Receiver<Reply<E>>,
    pub(crate) actions: Vec<Option<E::Action>>,
    pub(crate) episodes: Vec<Option<Box<E>>>,
    pub(crate) finished: Vec<Option<E::Finished>>,
}

impl<E: Episode> Round<E> {
    /// Room for `capacity` jobs in flight.
    pub(crate) fn new(capacity: usize) -> Self {
        let (reply, replies) = sync_channel(capacity);
        Self {
            reply,
            replies,
            actions: (0..capacity).map(|_| None).collect(),
            episodes: (0..capacity).map(|_| None).collect(),
            finished: (0..capacity).map(|_| None).collect(),
        }
    }

    /// Takes exactly `count` actions into `actions[..count]`; any other number is refused and
    /// leaves nothing staged. Counting stops a bound past `count`.
    pub(crate) fn stage(
        &mut self,
        actions: impl IntoIterator<Item = E::Action>,
        count: usize,
    ) -> Result<(), GymError> {
        let mut actions = actions.into_iter();
        let mut got = 0;
        for slot in &mut self.actions[..count] {
            *slot = actions.next();
            got += usize::from(slot.is_some());
        }
        if got == count {
            let extra = actions.take(MAX_QUEUE).count();
            if extra == 0 {
                return Ok(());
            }
            got += extra;
        }
        self.actions[..count]
            .iter_mut()
            .for_each(|slot| *slot = None);
        Err(GymError::ActionCount {
            expected: count,
            got,
        })
    }

    /// Waits for `count` replies and files them by position; the first failed position's
    /// error when any failed.
    pub(crate) fn receive(&mut self, count: usize) -> Result<(), E::Error> {
        let mut failure: Option<(usize, Failure<E::Error>)> = None;
        for _ in 0..count {
            let (position, done) = self
                .replies
                .recv()
                .expect("the round keeps a sender of its own");
            match done {
                Ok((episode, finished)) => {
                    assert!(self.episodes[position].is_none(), "one reply per position");
                    self.episodes[position] = Some(episode);
                    self.finished[position] = finished;
                }
                Err(error) => {
                    if failure.as_ref().is_none_or(|(first, _)| position < *first) {
                        failure = Some((position, error));
                    }
                }
            }
        }
        match failure {
            Some((_, Failure::Error(error))) => Err(error),
            Some((position, Failure::Panicked)) => Err(GymError::Panicked { position }.into()),
            Some((_, Failure::Undisposed)) => Err(GymError::Undisposed.into()),
            None => Ok(()),
        }
    }
}
