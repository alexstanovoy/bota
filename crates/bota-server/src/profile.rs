//! Sampled native phase timings and entity counts, emitted once per thread.

use std::cell::RefCell;
use std::io::{self, Write};
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Instant;

const PHASE_NAMES: [&str; 21] = [
    "tick",
    "upkeep",
    "effects",
    "stats",
    "targeting",
    "movement",
    "visibility",
    "actions",
    "damage",
    "projection",
    "settle",
    "target_query",
    "class_zero_query",
    "movement_intent",
    "walk",
    "separation",
    "body_index",
    "route",
    "path_query",
    "local_plan",
    "local_search",
];
const PHASE_COUNT: usize = PHASE_NAMES.len();
const SAMPLE_STRIDE: u32 = 16;

thread_local! {
    static RECORDER: RefCell<Recorder> = const { RefCell::new(Recorder {
        samples: [[0; 3]; PHASE_COUNT],
    }) };
}

struct Recorder {
    samples: [[u64; 3]; PHASE_COUNT],
}

/// Timed native regions; nested regions report inclusive durations.
#[repr(usize)]
#[derive(Clone, Copy)]
pub enum Phase {
    Tick,
    Upkeep,
    Effects,
    Stats,
    Targeting,
    Movement,
    Visibility,
    Actions,
    Damage,
    Projection,
    Settle,
    TargetQuery,
    ClassZeroQuery,
    MovementIntent,
    Walk,
    Separation,
    BodyIndex,
    Route,
    PathQuery,
    LocalPlan,
    LocalSearch,
}

const _: () = assert!(Phase::LocalSearch as usize + 1 == PHASE_COUNT);
const _: () = assert!(SAMPLE_STRIDE.is_power_of_two());

/// One sampled region, recording calls, elapsed nanoseconds and live-entity rows.
#[must_use]
pub struct ScopeGuard {
    phase: Phase,
    started: Option<(Instant, u64)>,
    _thread: PhantomData<Rc<()>>,
}

impl ScopeGuard {
    /// Starts a region on positive ticks divisible by sixteen; other ticks read no clock.
    pub fn new(phase: Phase, tick: u32, entities: usize) -> Self {
        Self {
            phase,
            _thread: PhantomData,
            started: sampled(tick).then(|| {
                let entities = u64::try_from(entities).expect("entity count fits u64");
                (Instant::now(), entities)
            }),
        }
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        let Some((started, entities)) = self.started else {
            return;
        };
        let elapsed = u64::try_from(started.elapsed().as_nanos())
            .expect("profile region duration fits u64 nanoseconds");
        RECORDER.with(|recorder| {
            let mut recorder = recorder.borrow_mut();
            let sample = &mut recorder.samples[self.phase as usize];
            for (total, value) in sample.iter_mut().zip([1, elapsed, entities]) {
                *total = total
                    .checked_add(value)
                    .expect("phase profile counter overflow");
            }
        });
    }
}

impl Recorder {
    fn emit(&self) -> io::Result<()> {
        let mut output = io::stderr().lock();
        write!(
            output,
            "bota-phase-profile-v1 stride={SAMPLE_STRIDE} phases="
        )?;
        for (index, (name, sample)) in PHASE_NAMES.iter().zip(&self.samples).enumerate() {
            if index != 0 {
                write!(output, ",")?;
            }
            write!(output, "{name}:{}:{}:{}", sample[0], sample[1], sample[2])?;
        }
        writeln!(output)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.samples.iter().any(|sample| sample[0] != 0) {
            let _ = self.emit();
        }
    }
}

/// Whether a simulation tick contributes to phase timing and query counts.
pub fn sampled(tick: u32) -> bool {
    tick != 0 && tick.is_multiple_of(SAMPLE_STRIDE)
}

/// Drains this thread's samples in Phase order as [calls, nanoseconds, live-entity rows].
pub fn take_snapshot() -> [[u64; 3]; PHASE_COUNT] {
    RECORDER.with(|recorder| {
        std::mem::replace(&mut recorder.borrow_mut().samples, [[0; 3]; PHASE_COUNT])
    })
}
