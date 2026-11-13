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
