//! The drivers' contracts under misuse and failure, on episodes that simulate nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::time::Duration;

use bota_gym::{Episode, GameId, GymError, Tournament, VecEnv, WorkerConfig, Workers};

#[derive(Debug, PartialEq, Eq)]
struct Error(GymError);

impl From<GymError> for Error {
    fn from(error: GymError) -> Self {
        Self(error)
    }
}

/// A panic payload whose own destructor panics.
struct Bomb;

impl Drop for Bomb {
    fn drop(&mut self) {
        panic!("the panic payload's destructor panics too");
    }
}

/// What a step does besides counting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Count,
    /// Panics with a payload that panics when dropped.
    Bomb,
}

/// A game that only counts its steps and lasts `plan` steps.
struct Steps {
    id: GameId,
    steps: u64,
    length: u64,
}

impl Episode for Steps {
    type Shared = ();
    type Plan = u64;
    type Action = Act;
    type Finished = u64;
    type Error = Error;

    fn start(_: &(), id: GameId, length: u64) -> Result<Self, Error> {
        Ok(Self {
            id,
            steps: 0,
            length,
        })
    }

    fn step(&mut self, _: &(), act: Act) -> Result<Option<u64>, Error> {
        if act == Act::Bomb {
            std::panic::panic_any(Bomb);
        }
        self.steps += 1;
        Ok((self.steps == self.length).then_some(self.steps))
    }

    fn game(&self) -> GameId {
        self.id
    }
}

fn one_worker() -> WorkerConfig {
    WorkerConfig {
        threads: 1,
        queue: 1,
        cpus: None,
        name: "drivers".to_owned(),
    }
}

fn steps(env: &VecEnv<'_, Steps>) -> Vec<u64> {
    env.episodes().iter().map(|episode| episode.steps).collect()
}

/// Runs `test` on its own thread and fails, rather than hangs, when it does not end in time.
/// The deadline is a hang guard only: a passing run ends in milliseconds.
fn within_a_minute(test: impl FnOnce() + Send + 'static) {
    let (done, finished) = channel();
    std::thread::spawn(move || {
        test();
        done.send(()).expect("the watchdog waits");
    });
    finished
        .recv_timeout(Duration::from_secs(60))
        .expect("the test finished: a driver hung or the test panicked");
}

#[test]
fn a_panic_whose_payload_panics_when_dropped_is_answered_and_the_worker_lives_on() {
    within_a_minute(|| {
        std::thread::scope(|scope| {
            let workers = Workers::spawn(scope, one_worker(), &()).expect("workers");
            let mut failing: VecEnv<Steps> =
                VecEnv::start(workers.clone(), vec![0, 1], &100).expect("started");
            assert_eq!(
                failing.step([Act::Count, Act::Bomb], &100, |_, _| {}),
                Err(Error(GymError::Panicked { position: 1 }))
            );
            // The only worker still serves another driver.
            let mut healthy: VecEnv<Steps> =
                VecEnv::start(workers, vec![2], &100).expect("started");
            healthy
                .step([Act::Count], &100, |_, _| {})
                .expect("the worker survived");
            assert_eq!(steps(&healthy), [1]);
        });
    });
}

/// Error conversions of [`Counted`], which only one test uses.
static CONVERSIONS: AtomicUsize = AtomicUsize::new(0);

/// An error type whose conversions are counted where the caller's code runs.
#[derive(Debug)]
struct Counted;

impl From<GymError> for Counted {
    fn from(_: GymError) -> Self {
        CONVERSIONS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

/// [`Steps`] with a counted error type.
struct Quiet(Steps);

impl Episode for Quiet {
    type Shared = ();
    type Plan = u64;
    type Action = ();
    type Finished = u64;
    type Error = Counted;

    fn start(_: &(), id: GameId, length: u64) -> Result<Self, Counted> {
        Ok(Self(Steps {
            id,
            steps: 0,
            length,
        }))
    }

    fn step(&mut self, _: &(), (): ()) -> Result<Option<u64>, Counted> {
        Ok(self.0.step(&(), Act::Count).expect("counting never fails"))
    }

    fn game(&self) -> GameId {
        self.0.id
    }
}

#[test]
fn a_successful_job_converts_no_error() {
    std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, one_worker(), &()).expect("workers");
        let mut env: VecEnv<Quiet> =
            VecEnv::start(workers.clone(), vec![0, 1], &3).expect("started");
        let mut tournament: Tournament<Quiet> =
            Tournament::new(workers, 1, [2, 2, 2], 2).expect("started");
        for _ in 0..4 {
            env.step([(); 2], &3, |_, _| {}).expect("a step");
        }
        while !tournament.is_over() {
            let count = tournament.live().len();
            tournament
                .step(vec![(); count], |_, _| Ok(()))
                .expect("a round");
        }
        assert_eq!(CONVERSIONS.load(Ordering::Relaxed), 0);
    });
}

#[test]
fn a_wrong_action_count_steps_no_episode_and_leaves_the_driver_usable() {
    std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, one_worker(), &()).expect("workers");
        let mut env: VecEnv<Steps> =
            VecEnv::start(workers.clone(), vec![0, 1], &100).expect("started");
        let short = env.step([Act::Count], &100, |_, _| {});
        assert_eq!(
            short,
            Err(Error(GymError::ActionCount {
                expected: 2,
                got: 1
            }))
        );
        let long = env.step([Act::Count; 3], &100, |_, _| {});
        assert_eq!(
            long,
            Err(Error(GymError::ActionCount {
                expected: 2,
                got: 3
            }))
        );
        assert_eq!(steps(&env), [0, 0], "no episode moved");
        env.step([Act::Count; 2], &100, |_, _| {})
            .expect("a full batch after a refused one");
        assert_eq!(steps(&env), [1, 1]);

        let mut tournament: Tournament<Steps> =
            Tournament::new(workers, 1, [5, 5], 2).expect("started");
        assert_eq!(
            tournament.step([Act::Count], |_, _| Ok(())),
            Err(Error(GymError::ActionCount {
                expected: 2,
                got: 1
            }))
        );
        let moved: Vec<u64> = tournament.live().iter().map(|e| e.steps).collect();
        assert_eq!(moved, [0, 0]);
        tournament
            .step([Act::Count; 2], |_, _| Ok(()))
            .expect("a full batch after a refused one");
    });
}

#[test]
fn a_failure_in_the_middle_of_a_step_spends_the_driver() {
    std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, one_worker(), &()).expect("workers");
        let mut env: VecEnv<Steps> =
            VecEnv::start(workers.clone(), vec![0, 1], &100).expect("started");
        assert_eq!(
            env.step([Act::Bomb, Act::Count], &100, |_, _| {}),
            Err(Error(GymError::Panicked { position: 0 }))
        );
        for _ in 0..2 {
            assert_eq!(
                env.step([Act::Count; 2], &100, |_, _| {}),
                Err(Error(GymError::Poisoned))
            );
        }
        let mut tournament: Tournament<Steps> =
            Tournament::new(workers, 1, [5, 5], 2).expect("started");
        assert_eq!(
            tournament.step([Act::Count, Act::Bomb], |_, _| Ok(())),
            Err(Error(GymError::Panicked { position: 1 }))
        );
        assert_eq!(
            tournament.step([Act::Count; 2], |_, _| Ok(())),
            Err(Error(GymError::Poisoned))
        );
    });
}

/// A game that panics on every step with its action as the payload.
struct Thrower(GameId);

impl Episode for Thrower {
    type Shared = ();
    type Plan = ();
    type Action = std::sync::Arc<Vec<u8>>;
    type Finished = ();
    type Error = Error;

    fn start(_: &(), id: GameId, (): ()) -> Result<Self, Error> {
        Ok(Self(id))
    }

    fn step(&mut self, _: &(), payload: std::sync::Arc<Vec<u8>>) -> Result<Option<()>, Error> {
        std::panic::panic_any(payload)
    }

    fn game(&self) -> GameId {
        self.0
    }
}

#[test]
fn an_ordinary_panic_payload_is_freed_before_the_driver_hears_of_it() {
    std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, one_worker(), &()).expect("workers");
        for _ in 0..16 {
            let payload = std::sync::Arc::new(vec![7u8; 1 << 20]);
            let witness = std::sync::Arc::downgrade(&payload);
            let mut env: VecEnv<Thrower> =
                VecEnv::start(workers.clone(), vec![0], &()).expect("started");
            assert_eq!(
                env.step([payload], &(), |_, _| {}),
                Err(Error(GymError::Panicked { position: 0 }))
            );
            assert!(witness.upgrade().is_none(), "the payload was dropped");
        }
    });
}

/// A panic payload whose destructor panics again, with the data it holds as that panic's
/// payload: an ordinary one.
struct Twice(Option<Arc<Vec<u8>>>);

impl Drop for Twice {
    fn drop(&mut self) {
        std::panic::panic_any(self.0.take());
    }
}

/// A panic payload whose destructor panics with a [`Twice`]: its second payload panics too.
struct Thrice(Option<Arc<Vec<u8>>>);

impl Drop for Thrice {
    fn drop(&mut self) {
        std::panic::panic_any(Twice(self.0.take()));
    }
}

/// A game whose every step panics with a [`Twice`] of its action; the shared counter counts the
/// steps that ran.
struct Relay(GameId);

impl Episode for Relay {
    type Shared = AtomicUsize;
    type Plan = ();
    type Action = Arc<Vec<u8>>;
    type Finished = ();
    type Error = Error;

    fn start(_: &AtomicUsize, id: GameId, (): ()) -> Result<Self, Error> {
        Ok(Self(id))
    }

    fn step(&mut self, ran: &AtomicUsize, data: Arc<Vec<u8>>) -> Result<Option<()>, Error> {
        ran.fetch_add(1, Ordering::Relaxed);
        std::panic::panic_any(Twice(Some(data)))
    }

    fn game(&self) -> GameId {
        self.0
    }
}

/// Data a payload carries, and witnesses of whether it still lives.
type Tracked = (Vec<Arc<Vec<u8>>>, Vec<std::sync::Weak<Vec<u8>>>);

fn megabytes(count: usize) -> Tracked {
    let data: Vec<_> = (0..count).map(|_| Arc::new(vec![9u8; 1 << 20])).collect();
    let witnesses = data.iter().map(Arc::downgrade).collect();
    (data, witnesses)
}

fn live(witnesses: &[std::sync::Weak<Vec<u8>>]) -> usize {
    witnesses
        .iter()
        .filter(|witness| witness.upgrade().is_some())
        .count()
}

#[test]
fn a_destructors_ordinary_second_payload_is_dropped_too() {
    let ran = AtomicUsize::new(0);
    let (data, witnesses) = megabytes(16);
    std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, one_worker(), &ran).expect("workers");
        let mut env: VecEnv<Relay> =
            VecEnv::start(workers.clone(), (0..16).collect(), &()).expect("started");
        assert_eq!(
            env.step(data, &(), |_, _| {}),
            Err(Error(GymError::Panicked { position: 0 }))
        );
        // Nothing was kept, so the pool still takes jobs.
        assert!(VecEnv::<Relay>::start(workers, vec![0], &()).is_ok());
    });
    assert_eq!(ran.load(Ordering::Relaxed), 16);
    assert_eq!(live(&witnesses), 0);
}

/// Times an [`Armed`] action's destructor ran with its data still in it.
static ARMED_DROPS: AtomicUsize = AtomicUsize::new(0);

/// An action whose destructor, while it still holds its data, panics three levels deep (with a
/// [`Thrice`]): a step that runs takes the data out first.
struct Armed(Option<Arc<Vec<u8>>>);

impl Drop for Armed {
    fn drop(&mut self) {
        if let Some(data) = self.0.take() {
            ARMED_DROPS.fetch_add(1, Ordering::Relaxed);
            std::panic::panic_any(Thrice(Some(data)));
        }
    }
}

/// What a [`Fuse`] game reads: the steps that ran, the round's size, and how many plans the
/// driver cloned since it was armed (one per submitted step).
#[derive(Default)]
struct Gate {
    ran: AtomicUsize,
    round: AtomicUsize,
    armed: AtomicBool,
    cloned: AtomicUsize,
}

/// A plan whose clones, once its gate is armed, count the driver's submissions.
struct Ticket(Arc<Gate>);

impl Clone for Ticket {
    fn clone(&self) -> Self {
        if self.0.armed.load(Ordering::SeqCst) {
            self.0.cloned.fetch_add(1, Ordering::SeqCst);
        }
        Self(Arc::clone(&self.0))
    }
}

/// A game whose step waits until the driver submitted every step of the round, then panics
/// three levels deep with its action's data.
struct Fuse(GameId);

impl Episode for Fuse {
    type Shared = Gate;
    type Plan = Ticket;
    type Action = Armed;
    type Finished = ();
    type Error = Error;

    fn start(_: &Gate, id: GameId, _: Ticket) -> Result<Self, Error> {
        Ok(Self(id))
    }

    fn step(&mut self, gate: &Gate, mut action: Armed) -> Result<Option<()>, Error> {
        gate.ran.fetch_add(1, Ordering::SeqCst);
        // Bounded by the test's watchdog: the driver submits the whole round promptly.
        while gate.cloned.load(Ordering::SeqCst) < gate.round.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        let data = action.0.take();
        std::panic::panic_any(Thrice(data))
    }

    fn game(&self) -> GameId {
        self.0
    }
}

/// Plays one step of `jobs` [`Fuse`] slots on `threads` workers; every step waits until all
/// `jobs` are submitted. The last action holds no data and a harmless destructor, since it may
/// be submitted after the pool turned terminal and be refused on this thread. Returns the steps
/// that ran and the data still alive once the pool is gone.
fn fuse(threads: usize, jobs: usize) -> (usize, usize) {
    let gate = Arc::new(Gate::default());
    let (data, witnesses) = megabytes(jobs - 1);
    std::thread::scope(|scope| {
        let config = WorkerConfig {
            threads,
            queue: jobs,
            ..one_worker()
        };
        let workers = Workers::spawn(scope, config, &*gate).expect("workers");
        let ticket = Ticket(Arc::clone(&gate));
        let mut env: VecEnv<Fuse> =
            VecEnv::start(workers.clone(), (0..jobs).collect(), &ticket).expect("started");
        gate.round.store(jobs, Ordering::SeqCst);
        gate.armed.store(true, Ordering::SeqCst);
        let actions = data
            .into_iter()
            .map(|data| Armed(Some(data)))
            .chain([Armed(None)]);
        // Every job is answered: the first failing position's error comes back. With several
        // workers the first position's job may be answered unrun, if another worker's panic made
        // the pool terminal before that job's worker looked.
        let stepped = env.step(actions, &ticket, |_, _| {});
        assert!(matches!(
            stepped,
            Err(Error(GymError::Panicked { .. } | GymError::Undisposed))
        ));
        if threads == 1 {
            assert_eq!(stepped, Err(Error(GymError::Panicked { position: 0 })));
        }
        assert_eq!(
            VecEnv::<Fuse>::start(workers, vec![0], &ticket).err(),
            Some(Error(GymError::Undisposed))
        );
    });
    (gate.ran.load(Ordering::SeqCst), live(&witnesses))
}

#[test]
fn the_first_undroppable_payload_stops_the_pool_and_leaks_the_queued_jobs_whole() {
    within_a_minute(|| {
        let (ran, alive) = fuse(1, 32);
        assert_eq!(ran, 1, "nothing runs once the pool turned terminal");
        assert_eq!(
            ARMED_DROPS.load(Ordering::SeqCst),
            0,
            "no rejected job was dropped"
        );
        // The documented bound, met exactly: one kept payload for the one worker that ran a
        // job, plus the 30 jobs with data queued when the pool turned terminal, leaked whole.
        assert_eq!(alive, 1 + 30);
        assert_eq!(
            GymError::Undisposed.to_string(),
            "the workers met a panic payload they could not drop, and run no more jobs"
        );
    });
}

#[test]
fn several_workers_run_at_most_one_job_each_before_the_pool_turns_terminal() {
    within_a_minute(|| {
        let threads = 4;
        let (ran, alive) = fuse(threads, 64);
        // At most one job per worker ran: those a worker took before the pool turned terminal.
        assert!((1..=threads).contains(&ran), "{ran} jobs ran");
        assert_eq!(
            ARMED_DROPS.load(Ordering::SeqCst),
            0,
            "no rejected job was dropped"
        );
        // Every job with data either ran and kept its payload, or was leaked whole.
        assert_eq!(alive, 63);
    });
}

/// Panics whenever a panic is turned into it: caller code running on the driver's thread.
#[derive(Debug, PartialEq, Eq)]
struct Touchy(GymError);

impl From<GymError> for Touchy {
    fn from(error: GymError) -> Self {
        assert!(
            !matches!(error, GymError::Panicked { .. }),
            "the conversion panics"
        );
        Self(error)
    }
}

/// [`Steps`] whose error converts with a panic.
struct TouchySteps(Steps);

impl Episode for TouchySteps {
    type Shared = ();
    type Plan = u64;
    type Action = Act;
    type Finished = u64;
    type Error = Touchy;

    fn start(_: &(), id: GameId, length: u64) -> Result<Self, Touchy> {
        Ok(Self(Steps {
            id,
            steps: 0,
            length,
        }))
    }

    fn step(&mut self, _: &(), act: Act) -> Result<Option<u64>, Touchy> {
        self.0.step(&(), act).map_err(|Error(error)| Touchy(error))
    }

    fn game(&self) -> GameId {
        self.0.id
    }
}

fn unwinds(step: impl FnOnce()) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)).is_err()
}

#[test]
fn a_driver_whose_callers_code_unwinds_mid_step_is_spent() {
    std::thread::scope(|scope| {
        let workers = Workers::spawn(scope, one_worker(), &()).expect("workers");
        // The conversion of the worker's panic report unwinds on the driver's thread.
        let mut env: VecEnv<TouchySteps> =
            VecEnv::start(workers.clone(), vec![0, 1], &100).expect("started");
        assert!(unwinds(|| drop(env.step(
            [Act::Bomb, Act::Count],
            &100,
            |_, _| {}
        ))));
        assert_eq!(
            env.step([Act::Count; 2], &100, |_, _| {}),
            Err(Touchy(GymError::Poisoned))
        );
        let mut tournament: Tournament<TouchySteps> =
            Tournament::new(workers.clone(), 0, [100, 100], 2).expect("started");
        assert!(unwinds(|| drop(
            tournament.step([Act::Bomb, Act::Count], |_, _| Ok(()))
        )));
        assert_eq!(
            tournament.step([Act::Count; 2], |_, _| Ok(())),
            Err(Touchy(GymError::Poisoned))
        );
        // A completion callback unwinds.
        let mut env: VecEnv<TouchySteps> =
            VecEnv::start(workers.clone(), vec![0, 1], &1).expect("started");
        assert!(unwinds(|| {
            drop(env.step([Act::Count; 2], &1, |_, _: u64| -> () {
                panic!("booking fails")
            }))
        }));
        assert_eq!(
            env.step([Act::Count; 2], &1, |_, _| {}),
            Err(Touchy(GymError::Poisoned))
        );
        let mut tournament: Tournament<TouchySteps> =
            Tournament::new(workers, 0, [1, 1], 2).expect("started");
        assert!(unwinds(|| {
            drop(
                tournament.step([Act::Count; 2], |_, _| -> Result<(), Touchy> {
                    panic!("booking fails")
                }),
            )
        }));
        assert_eq!(
            tournament.step([Act::Count; 2], |_, _| Ok(())),
            Err(Touchy(GymError::Poisoned))
        );
    });
}

/// A plan that holds a handle to the pool its games run on.
struct Owning<'scope> {
    pool: Workers<'scope, Captive<'scope>>,
    gate: &'scope Gate,
}

impl Clone for Owning<'_> {
    fn clone(&self) -> Self {
        if self.gate.armed.load(Ordering::SeqCst) {
            self.gate.cloned.fetch_add(1, Ordering::SeqCst);
        }
        Self {
            pool: self.pool.clone(),
            gate: self.gate,
        }
    }
}

/// A game that holds a handle to its own pool; with `true` its step waits for the whole round to
/// be submitted, then panics three levels deep.
struct Captive<'scope> {
    id: GameId,
    _pool: Workers<'scope, Captive<'scope>>,
}

impl<'scope> Episode for Captive<'scope> {
    type Shared = Gate;
    type Plan = Owning<'scope>;
    type Action = bool;
    type Finished = ();
    type Error = Error;

    fn start(_: &Gate, id: GameId, plan: Owning<'scope>) -> Result<Self, Error> {
        Ok(Self {
            id,
            _pool: plan.pool,
        })
    }

    fn step(&mut self, gate: &Gate, fail: bool) -> Result<Option<()>, Error> {
        if fail {
            while gate.cloned.load(Ordering::SeqCst) < gate.round.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            std::panic::panic_any(Thrice(None));
        }
        Ok(None)
    }

    fn game(&self) -> GameId {
        self.id
    }
}

/// Plays one round of three games that hold handles to their own pool, the first failing when
/// `fail`, then drops every handle the caller owns: the pool must shut down all the same.
fn shut_down_with_captive_handles(fail: bool) {
    within_a_minute(move || {
        let gate = Gate::default();
        std::thread::scope(|scope| {
            let config = WorkerConfig {
                queue: 3,
                ..one_worker()
            };
            let workers = Workers::spawn(scope, config, &gate).expect("workers");
            let plan = Owning {
                pool: workers.clone(),
                gate: &gate,
            };
            let mut env: VecEnv<Captive> =
                VecEnv::start(workers, vec![0, 1, 2], &plan).expect("started");
            gate.round.store(3, Ordering::SeqCst);
            gate.armed.store(true, Ordering::SeqCst);
            let stepped = env.step([fail, false, false], &plan, |_, _| {});
            match fail {
                true => assert_eq!(stepped, Err(Error(GymError::Panicked { position: 0 }))),
                false => assert_eq!(stepped, Ok(())),
            }
            drop(env);
            drop(plan);
        });
    });
}

#[test]
fn a_terminal_pool_shuts_down_although_leaked_jobs_hold_handles_to_it() {
    shut_down_with_captive_handles(true);
}

#[test]
fn a_healthy_pool_shuts_down_once_its_games_and_handles_are_dropped() {
    shut_down_with_captive_handles(false);
}
