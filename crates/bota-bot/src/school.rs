//! Teaching the model a lesson.
//!
//! A round is: play some matches with the model choosing loosely, credit every
//! decision with what followed it, and move the weights towards the choices
//! that were followed by more than the model expected. Then measure what it
//! has become, on the same matches every time, and keep it only if it is
//! better than the best so far.
//!
//! Three things here are answers to what the first bot got wrong.
//!
//! **The baseline is learned.** How good a decision was is what followed it
//! less what was expected of that position, and expecting is the value head's
//! job. The first bot had no value head and had to invent baselines — the
//! match's own score, then the average over decisions at the same point on the
//! clock — and they were measured against each other and came out a tie. What
//! a tie was pointing at is that neither was a baseline.
//!
//! **What is measured does not move.** The same seeds every round, the model
//! choosing greedily, and the number reported is what the lesson paid. A
//! round's own matches are worth whatever their seeds were worth, so reading
//! that as progress is reading the luck of the draw.
//!
//! **Only what measures better is kept.** Practising wanders as much as it
//! climbs, and a run that writes out its last weights rather than its best
//! throws away the good ones on the way past.

use std::path::PathBuf;
use std::thread;

use crate::{Adam, Chair, DEEDS, Dice, INPUT, Learned, Lesson, Model, Role, Roll, Student, Yard};

/// How a lesson is taught.
#[derive(Clone, Debug)]
pub struct School {
    /// Where the matches are played.
    pub yard: Yard,
    /// Which lesson.
    pub lesson: Lesson,
    /// What the seats are there to do.
    pub role: Role,
    /// Rounds to run.
    pub rounds: u32,
    /// Matches a round.
    pub matches: usize,
    /// How many run at once.
    pub lanes: usize,
    /// How loosely it chooses while learning.
    pub heat: f32,
    /// How far a step goes.
    pub rate: f32,
    /// Decisions added up before the weights move.
    pub batch: usize,
    /// The most decisions of one round the weights are moved by.
    pub most_frames: usize,
    /// How much of what a decision led to is still counted a tick later.
    pub discount: f32,
    /// Ticks a decision is credited with.
    pub window: usize,
    /// Rounds between measuring.
    pub measure_every: u32,
    /// Where the search is seeded from.
    pub seed: u64,
    /// Where the weights are kept.
    pub weights: PathBuf,
}

impl School {
    /// A school with the plain settings, for a lesson.
    pub fn for_lesson(lesson: Lesson) -> School {
        School {
            yard: Yard::default(),
            lesson,
            role: Role::Mid,
            rounds: 20,
            matches: 8,
            lanes: 8,
            heat: 1.0,
            rate: 3e-4,
            batch: 64,
            most_frames: 8000,
            discount: 0.997,
            window: 900,
            measure_every: 5,
            seed: 1,
            weights: Model::path(),
        }
    }
}

/// Plays one round of matches and gathers what happened.
///
/// Both seats are the model, which is the only opponent there is: what a
/// lesson pays for is not beating anybody, so who is on the other side matters
/// less here than it will later.
pub fn gather(how: &School, weights: &[u8], round: u64) -> std::io::Result<Vec<Roll>> {
    let seeds: Vec<u64> = (0..how.matches)
        .map(|at| {
            how.seed
                .wrapping_mul(0x9e37_79b9)
                .wrapping_add(round.wrapping_mul(1013))
                .wrapping_add(at as u64)
        })
        .collect();
    let mut rolls = Vec::new();
    for lane in seeds.chunks(how.lanes.max(1)) {
        let played: Vec<std::io::Result<Vec<Roll>>> = thread::scope(|scope| {
            let running: Vec<_> = lane
                .iter()
                .map(|seed| {
                    let seed = *seed;
                    scope.spawn(move || one_match(how, weights, seed, how.heat))
                })
                .collect();
            running
                .into_iter()
                .map(|one| {
                    one.join()
                        .unwrap_or_else(|_| Err(std::io::Error::other("a match gave up")))
                })
                .collect()
        });
        for outcome in played {
            rolls.extend(outcome?);
        }
    }
    Ok(rolls)
}

/// One match of the model against itself, both seats writing down what they
/// did.
fn one_match(how: &School, weights: &[u8], seed: u64, heat: f32) -> std::io::Result<Vec<Roll>> {
    let hatch = |seed: u64| -> std::io::Result<Student<Learned>> {
        let model = Model::from_bytes(weights, how.seed).map_err(std::io::Error::other)?;
        Ok(Student::new(Learned::loosely(model, heat, seed)))
    };
    let mut one = hatch(seed)?;
    let mut other = hatch(seed ^ 0x5bf0_3635)?;
    let chair = |name: &str| Chair {
        addr: String::new(),
        name: name.to_string(),
        hero: how.yard.hero,
        limit: Some(how.lesson.ticks()),
        role: how.role,
        lesson: how.lesson,
        until: None,
    };
    how.yard
        .play_a_match(seed, &mut one, &mut other, &chair("one"), &chair("other"))?;
    let mut out = Vec::new();
    for student in [one, other] {
        let mut roll = student.roll;
        roll.settle(how.discount, how.window);
        out.push(roll);
    }
    Ok(out)
}

/// What the model is worth at this lesson, on matches that never change.
///
/// Choosing greedily, so that what is measured is what it believes rather than
/// what it happened to draw.
pub fn measure(how: &School, weights: &[u8]) -> std::io::Result<f32> {
    let steady = School {
        matches: MEASURED_ON,
        heat: 0.0,
        seed: 0x5eed_0001,
        ..how.clone()
    };
    let rolls = gather(&steady, weights, 0)?;
    if rolls.is_empty() {
        return Ok(0.0);
    }
    Ok(rolls.iter().map(|roll| roll.paid_in_all()).sum::<f32>() / rolls.len() as f32)
}

/// Matches the measuring is done over.
const MEASURED_ON: usize = 6;

/// What one round came to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Round {
    /// Which round.
    pub number: u32,
    /// Decisions gone over.
    pub frames: usize,
    /// What the lesson paid over the round's own matches, per match.
    pub paid: f32,
    /// The loss over the round.
    pub loss: f32,
    /// What it measured, on the rounds that measured.
    pub measured: Option<f32>,
    /// Whether it was kept.
    pub kept: bool,
}

/// Moves the weights towards the choices that were followed by more than the
/// model expected of the position they were made in.
pub fn learn_from(
    model: &Model,
    adam: &mut Adam,
    frames: &[crate::Frame],
    how: &School,
    dice: &mut Dice,
) -> f32 {
    let mut order: Vec<usize> = (0..frames.len()).collect();
    for at in (1..order.len()).rev() {
        order.swap(at, (dice.next_u64() % (at as u64 + 1)) as usize);
    }
    let mut total = 0.0;
    let mut batches = 0.0;
    for batch in order.chunks(how.batch.max(1)) {
        let mut rows = Vec::with_capacity(batch.len() * INPUT);
        let mut chosen = Vec::with_capacity(batch.len());
        let mut worths = Vec::with_capacity(batch.len());
        let mut masks = Vec::with_capacity(batch.len() * DEEDS);
        for at in batch {
            let frame = &frames[*at];
            rows.extend_from_slice(&frame.numbers);
            chosen.push(frame.chosen as u32);
            worths.push(frame.worth);
            masks.extend(frame.allowed.iter().map(|may| if *may { 0.0 } else { 1.0 }));
        }
        let Ok(loss) = crate::a_step(model, &rows, &chosen, &worths, &masks, batch.len()) else {
            continue;
        };
        if let Ok(number) = loss.flatten_all().and_then(|one| one.to_vec1::<f32>()) {
            total += number.first().copied().unwrap_or(0.0);
            batches += 1.0;
        }
        let _ = adam.step(model, &loss);
    }
    if batches > 0.0 { total / batches } else { 0.0 }
}
