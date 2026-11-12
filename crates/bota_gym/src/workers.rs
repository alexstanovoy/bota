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
//! [`GymError::Panicked`] for that position; the episode is lost and its driver stops, while
//! the workers and other drivers carry on.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

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

/// One job's result: the episode (its next game's, when a game ended and a plan was given) and
/// what a finished game left.
pub(crate) type Done<E> = Result<(Box<E>, Option<<E as Episode>::Finished>), <E as Episode>::Error>;
pub(crate) type Reply<E> = (usize, Done<E>);

struct Envelope<'scope, E: Episode> {
    position: usize,
    job: Job<'scope, E>,
    reply: SyncSender<Reply<E>>,
}

/// A handle to shared simulation workers; they leave once every handle is dropped.
pub struct Workers<'scope, E: Episode> {
    jobs: SyncSender<Envelope<'scope, E>>,
}

impl<E: Episode> Clone for Workers<'_, E> {
    fn clone(&self) -> Self {
        Self {
            jobs: self.jobs.clone(),
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
        for index in 0..config.threads {
            let receiver = Arc::clone(&receiver);
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
                    work(&receiver, shared);
                })
                .map_err(|_| GymError::Config("a worker thread could not be spawned"))?;
        }
        Ok(Self { jobs })
    }

    pub(crate) fn submit(
        &self,
        position: usize,
        job: Job<'scope, E>,
        reply: &SyncSender<Reply<E>>,
    ) -> Result<(), GymError> {
        let envelope = Envelope {
            position,
            job,
            reply: reply.clone(),
        };
        // Waits while the queue is full: the backpressure every driver shares.
        self.jobs.send(envelope).map_err(|_| GymError::Stopped)
    }
}

fn work<E: Episode>(receiver: &Mutex<Receiver<Envelope<'_, E>>>, shared: &E::Shared) {
    loop {
        let envelope = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(Envelope {
            position,
            job,
            reply,
        }) = envelope
        else {
            return;
        };
        let done = catch_unwind(AssertUnwindSafe(|| run(job, shared)))
            .unwrap_or(Err(GymError::Panicked { position }.into()));
        // A driver that failed no longer reads its replies.
        let _ = reply.send((position, done));
    }
}

fn run<E: Episode>(job: Job<'_, E>, shared: &E::Shared) -> Done<E> {
    match job {
        Job::Start { game, plan } => start(shared, game, plan).map(|episode| (episode, None)),
        Job::Build { slot, build } => {
            let episode = build(shared)?;
            if episode.game().slot != slot {
                return Err(GymError::GameMismatch.into());
            }
            Ok((Box::new(episode), None))
        }
        Job::Step {
            mut episode,
            action,
            next,
        } => {
            let finished = episode.step(shared, action)?;
            match (&finished, next) {
                (Some(_), Some(plan)) => episode
                    .game()
                    .next()
                    .map_err(E::Error::from)
                    .and_then(|game| start(shared, game, plan))
                    .map(|next| (next, finished)),
                _ => Ok((episode, finished)),
            }
        }
    }
}

fn start<E: Episode>(shared: &E::Shared, game: GameId, plan: E::Plan) -> Result<Box<E>, E::Error> {
    let episode = E::start(shared, game, plan)?;
    if episode.game() != game {
        return Err(GymError::GameMismatch.into());
    }
    Ok(Box::new(episode))
}

/// A driver's side of one round: its reply channel and the slots replies land in.
pub(crate) struct Round<E: Episode> {
    pub(crate) reply: SyncSender<Reply<E>>,
    replies: Receiver<Reply<E>>,
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
            episodes: (0..capacity).map(|_| None).collect(),
            finished: (0..capacity).map(|_| None).collect(),
        }
    }

    /// Waits for `count` replies and files them by position; the first failed position's
    /// error when any failed.
    pub(crate) fn receive(&mut self, count: usize) -> Result<(), E::Error> {
        let mut failure: Option<(usize, E::Error)> = None;
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
            Some((_, error)) => Err(error),
            None => Ok(()),
        }
    }
}
