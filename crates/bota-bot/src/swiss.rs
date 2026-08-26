//! Choosing a crowd by tournament rather than by its own company.
//!
//! A generation plays a few rounds against crowd-mates paired by standing,
//! then one anchor match against the crowd's incoming best. What a model is
//! worth is its margin: its card less its opponent's, added up over every
//! match it played. A pair plays its round's seed twice, once from either
//! side; the anchor match is played once, from the same side by everybody.

use std::thread;

use crate::{Body, Card, Lesson, Rung, Tribe, pitted};

/// How a generation is chosen over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// Every model plays itself, and its two seats are averaged.
    Mirror,
    /// A tournament: rounds paired by standing, scored by margins.
    Swiss,
}

impl Selection {
    /// How a plan spells it.
    pub fn spelling(self) -> &'static str {
        match self {
            Selection::Mirror => "mirror",
            Selection::Swiss => "swiss",
        }
    }

    /// The selection a spelling names.
    pub fn named(name: &str) -> Option<Selection> {
        [Selection::Mirror, Selection::Swiss]
            .into_iter()
            .find(|way| way.spelling() == name)
    }
}

/// Rounds a tournament generation plays before the anchor match.
pub const SWISS_ROUNDS: usize = 3;
/// How many of the first rounds run on the shorter clock.
pub const SWISS_SHORT_ROUNDS: usize = 2;
/// How many times shorter than the stage's clock those rounds are.
pub const SHORT_SHARE: u32 = 4;

/// The crowd in standing order, best first.
///
/// Ties keep the order they arrived in, so two models worth the same never
/// swap places and a run stays repeatable.
pub fn standings(margins: &[f32]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..margins.len()).collect();
    order.sort_by(|one, other| {
        margins[*other]
            .partial_cmp(&margins[*one])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(one.cmp(other))
    });
    order
}

/// Neighbours in the standing paired off: first against second, third
/// against fourth, and so on. An odd crowd's last standing sits the round
/// out.
pub fn pairs_of(order: &[usize]) -> Vec<(usize, usize)> {
    order
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

/// What one tournament generation came to: a margin per model, and how many
/// matches never finished.
///
/// Every round is one fresh seed, common to all its pairs. The first
/// [`SWISS_SHORT_ROUNDS`] rounds run on a [`SHORT_SHARE`]th of the stage's
/// clock, the rest on the whole of it. The anchor match runs on the whole
/// clock against the crowd's incoming best, and moves only the challenger's
/// margin.
///
/// A match that fails counts as nothing for either side, and comes back in
/// the failure count; every match failing is reported as an error instead.
pub fn swiss_margins(
    tribe: &Tribe,
    crowd: &[Body],
    rung: &Rung,
    life: u32,
) -> std::io::Result<(Vec<f32>, usize)> {
    let seeds = tribe.round_seeds(life);
    let mut margins = vec![0.0f32; crowd.len()];
    let mut played = 0;
    let mut failed = 0;
    let mut last_words = None;

    for (round, seed) in seeds.iter().enumerate().take(SWISS_ROUNDS) {
        let clock = if round < SWISS_SHORT_ROUNDS {
            (rung.ticks / SHORT_SHARE).max(1)
        } else {
            rung.ticks
        };
        let short = Rung {
            ticks: clock,
            ..*rung
        };
        let jobs: Vec<(usize, usize)> = pairs_of(&standings(&margins))
            .into_iter()
            .flat_map(|(one, other)| [(one, other), (other, one)])
            .collect();
        let outcomes = bouts(tribe, crowd, &jobs, &short, *seed);
        let (more, lost, words) = settled(outcomes, rung.lesson, true, &mut margins);
        played += more;
        failed += lost;
        if words.is_some() {
            last_words = words;
        }
    }

    let challenges: Vec<(usize, usize)> = (0..crowd.len()).map(|at| (at, 0)).collect();
    let outcomes = bouts(tribe, crowd, &challenges, rung, seeds[SWISS_ROUNDS]);
    let (more, lost, words) = settled(outcomes, rung.lesson, false, &mut margins);
    played += more;
    failed += lost;
    if words.is_some() {
        last_words = words;
    }

    if played > 0
        && failed == played
        && let Some(wrong) = last_words
    {
        return Err(wrong);
    }
    Ok((margins, failed))
}

/// Settles a set of bouts into the margins, and says how many were played,
/// how many failed, and what the last failure said.
///
/// `both` says whether the second seat's margin moves too; on the anchor
/// match it does not.
pub fn settled(
    outcomes: Vec<std::io::Result<(usize, usize, Card, Card)>>,
    lesson: Lesson,
    both: bool,
    margins: &mut [f32],
) -> (usize, usize, Option<std::io::Error>) {
    let mut played = 0;
    let mut failed = 0;
    let mut last_words = None;
    for outcome in outcomes {
        played += 1;
        match outcome {
            Ok((one, other, mine, theirs)) => {
                let margin = mine.of(lesson) - theirs.of(lesson);
                margins[one] += margin;
                if both {
                    margins[other] -= margin;
                }
            }
            Err(wrong) => {
                failed += 1;
                last_words = Some(wrong);
            }
        }
    }
    (played, failed, last_words)
}

/// Plays a set of pairings on one seed, as many at a time as there are
/// lanes.
///
/// Each outcome carries the pairing it came from, so how the work was
/// shared out cannot change the answer.
fn bouts(
    tribe: &Tribe,
    crowd: &[Body],
    jobs: &[(usize, usize)],
    rung: &Rung,
    seed: u64,
) -> Vec<std::io::Result<(usize, usize, Card, Card)>> {
    let mut out = Vec::with_capacity(jobs.len());
    for batch in jobs.chunks(tribe.lanes.max(1)) {
        let done: Vec<std::io::Result<(usize, usize, Card, Card)>> = thread::scope(|scope| {
            let running: Vec<_> = batch
                .iter()
                .map(|(one, other)| {
                    let (one, other) = (*one, *other);
                    scope.spawn(move || {
                        pitted(tribe, &crowd[one], &crowd[other], rung, seed)
                            .map(|(mine, theirs)| (one, other, mine, theirs))
                    })
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
        out.extend(done);
    }
    out
}
