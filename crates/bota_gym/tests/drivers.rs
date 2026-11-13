//! The drivers' contracts under misuse and failure, on episodes that simulate nothing.

use std::sync::atomic::{AtomicUsize, Ordering};
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

#[test]
fn payloads_that_cannot_be_dropped_are_bounded_and_then_refused() {
    within_a_minute(|| {
        std::thread::scope(|scope| {
            let workers = Workers::spawn(scope, one_worker(), &()).expect("workers");
            for _ in 0..bota_gym::workers::MAX_UNDISPOSED_PANICS {
                let mut env: VecEnv<Steps> =
                    VecEnv::start(workers.clone(), vec![0], &100).expect("started");
                assert_eq!(
                    env.step([Act::Bomb], &100, |_, _| {}),
                    Err(Error(GymError::Panicked { position: 0 }))
                );
            }
            let refused = VecEnv::<Steps>::start(workers, vec![0], &100).err();
            assert_eq!(refused, Some(Error(GymError::Undisposed)));
            assert_eq!(
                GymError::Undisposed.to_string(),
                "the workers kept too many panic payloads that panicked when dropped, and stopped"
            );
        });
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
