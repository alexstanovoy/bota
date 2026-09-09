//! Teaching by breeding rather than by gradient.
//!
//! A crowd of models plays the lesson, the best of them are kept, and the rest
//! of the crowd is refilled by copying those with noise added. No credit is
//! worked out for any single decision: what is scored is the match, which is
//! also what is reported, so the thing being improved and the thing being
//! measured cannot come apart.
//!
//! Three things about the shape of it.
//!
//! **The trial seeds move.** A crowd judged on the same handful of matches
//! every generation is a crowd being selected for those matches, and with two
//! hundred thousand numbers to play with it will learn them. The seeds are a
//! function of which generation it is, so a run is still repeatable to the
//! number while no model is ever asked twice to do well at the same match. A
//! separate set that never moves is used to report, and never to choose.
//!
//! **Ranks, not marks.** A lesson pays five for shopping and forty-five for a
//! wave, so choosing by the marks themselves would mean a different pressure
//! on every rung. Rank has no units.
//!
//! **The spread moves, on a leash.** More than a fifth of the children
//! beating their parents widens it, fewer narrows it: a search that keeps
//! failing is reaching too far, and one that nearly always succeeds is not
//! reaching far enough. What a plan writes is where it starts, and the rule
//! never takes it more than eightfold from there either way.
//!
//! **The crowd carries over.** A lesson ends with the same number of models it
//! started with, and the next lesson starts from those. What is inherited is a
//! crowd rather than a champion: a lesson's best is often narrow, and the one
//! behind it is what the next lesson turns out to want.

use std::thread;

use crate::{
    Card, Chair, Dice, Learned, Lesson, Model, Role, Rung, SWISS_ROUNDS, Selection, Yard,
    standings, swiss_margins,
};

/// A crowd being taught.
#[derive(Clone, Debug)]
pub struct Tribe {
    /// Where the matches are played.
    pub yard: Yard,
    /// What the seats are there to do.
    pub role: Role,
    /// How many models there are.
    pub folk: usize,
    /// Matches each model plays a generation.
    pub trials: usize,
    /// Generations a lesson runs for.
    pub lives: u32,
    /// How many of the crowd survive a generation.
    pub keep: usize,
    /// How far a child is moved from its parent.
    pub spread: f32,
    /// How many matches run at once.
    pub lanes: usize,
    /// How a generation is chosen over.
    pub selection: Selection,
    /// Where the whole run is seeded from.
    pub seed: u64,
}

/// What one model is worth at every lesson, from matches it never trained on.
///
/// One match a seed, run to the longest lesson's clock, and every lesson
/// scored off that one match: each counts the ticks inside its own window. The
/// whole ladder for the price of its longest rung, and a card that describes
/// one game rather than seven different ones.
pub fn report_card(tribe: &Tribe, body: &Body) -> std::io::Result<Card> {
    let longest = Lesson::longest().rung();
    let (cards, _) = worth_of_all(
        tribe,
        std::slice::from_ref(body),
        longest,
        &tribe.reported_on(),
    )?;
    Ok(cards.first().copied().unwrap_or_default())
}

/// Matches a lesson is reported on, which never change.
pub const REPORTED_ON: usize = 4;
/// Where the reporting seeds come from, apart from everything else.
const REPORTING_SEED: u64 = 0x5eed_0001;

impl Tribe {
    /// A crowd with the plain settings.
    pub fn new(folk: usize, trials: usize) -> Tribe {
        Tribe {
            yard: Yard::default(),
            role: Role::Mid,
            folk: folk.max(2),
            trials: trials.max(1),
            lives: 30,
            keep: (folk / 4).max(1),
            spread: 0.02,
            lanes: 12,
            selection: Selection::Mirror,
            seed: 1,
        }
    }

    /// The matches a generation is judged on.
    ///
    /// A function of which generation it is, so that a run repeats to the
    /// number and no model is ever judged twice on the same match.
    pub fn trials_of(&self, life: u32) -> Vec<u64> {
        let mut dice = Dice::from_seed(
            self.seed
                .wrapping_mul(0x9e37_79b9)
                .wrapping_add(u64::from(life).wrapping_mul(0x1000_0001)),
        );
        (0..self.trials).map(|_| dice.next_u64()).collect()
    }

    /// The matches a lesson is reported on, the same ones every time.
    pub fn reported_on(&self) -> Vec<u64> {
        let mut dice = Dice::from_seed(REPORTING_SEED);
        (0..REPORTED_ON).map(|_| dice.next_u64()).collect()
    }

    /// The seeds of a tournament generation's rounds, the anchor match's
    /// last.
    ///
    /// One a round, common to every pair of it, and a function of which
    /// generation it is, exactly as the trials are.
    pub fn round_seeds(&self, life: u32) -> Vec<u64> {
        let mut dice = Dice::from_seed(
            self.seed
                .wrapping_mul(0x0b0f_a577)
                .wrapping_add(u64::from(life).wrapping_mul(0x4000_0005)),
        );
        (0..=SWISS_ROUNDS).map(|_| dice.next_u64()).collect()
    }
}

/// One model's numbers.
pub type Body = Vec<f32>;

/// A crowd started from one kept body: the body itself first, and the rest
/// children moved off it, exactly as a generation refills.
///
/// How a run continues from weights it kept: the crowd starts where the last
/// run ended rather than from noise. The children are drawn as generation
/// nought, which no generation of the run itself uses.
pub fn crowd_from(tribe: &Tribe, body: Body) -> Vec<Body> {
    next_crowd(tribe, std::slice::from_ref(&body), &[0], 0)
}

/// A crowd drawn at random.
pub fn first_crowd(tribe: &Tribe) -> Result<Vec<Body>, String> {
    (0..tribe.folk)
        .map(|at| {
            Model::fresh(tribe.seed.wrapping_add(at as u64).wrapping_mul(31))
                .and_then(|model| model.pour())
                .map_err(|wrong| wrong.to_string())
        })
        .collect()
}

/// What two bodies came to against each other, each seat's card its own.
///
/// Both choose what they like best, so a bout between two bodies on one seed
/// always comes out the same.
pub fn pitted(
    tribe: &Tribe,
    one: &Body,
    other: &Body,
    rung: &Rung,
    seed: u64,
) -> std::io::Result<(Card, Card)> {
    let hatch = |body: &Body| -> std::io::Result<Learned> {
        let model = Model::fresh(1).map_err(std::io::Error::other)?;
        model.soak(body).map_err(std::io::Error::other)?;
        Ok(Learned::new(model))
    };
    let mut mine = hatch(one)?;
    let mut theirs = hatch(other)?;
    let chair = |name: &str| Chair {
        addr: String::new(),
        name: name.to_string(),
        hero: tribe.yard.hero,
        limit: Some(rung.ticks),
        role: tribe.role,
        lesson: rung.lesson,
        until: Some(rung.ticks),
    };
    let (played, they_played) =
        tribe
            .yard
            .play_a_match(seed, &mut mine, &mut theirs, &chair("one"), &chair("other"))?;
    Ok((played.card, they_played.card))
}

/// What one model is worth over one match.
///
/// Both seats are the model itself, choosing what it likes best. A lesson pays
/// for what a seat does rather than for beating anybody, so the two seats are
/// two readings of the same model rather than a contest.
fn worth_of(tribe: &Tribe, body: &Body, rung: &Rung, seed: u64) -> std::io::Result<Card> {
    let (mine, theirs) = pitted(tribe, body, body, rung, seed)?;
    let mut both = mine;
    both.add(&theirs);
    Ok(both.over(2))
}

/// What every model of a crowd is worth, averaged over the matches given.
///
/// Every model against every match, run as many at a time as there are lanes.
/// Results go back to the slot they came from, so how the work was shared out
/// cannot change the answer.
///
/// A match that fails counts as nothing for the model that was playing it, and
/// the number that failed comes back alongside the marks. One bad match out of
/// hundreds is not worth a run of several hours, but every match failing is
/// something else — no server, or none of them able to start — and that is
/// reported as an error rather than as a crowd worth nothing.
pub fn worth_of_all(
    tribe: &Tribe,
    crowd: &[Body],
    rung: &Rung,
    seeds: &[u64],
) -> std::io::Result<(Vec<Card>, usize)> {
    let jobs: Vec<(usize, u64)> = crowd
        .iter()
        .enumerate()
        .flat_map(|(at, _)| seeds.iter().map(move |seed| (at, *seed)))
        .collect();
    let mut marks = vec![Card::new(); crowd.len()];
    let mut failed = 0;
    let mut last_words = None;
    for batch in jobs.chunks(tribe.lanes.max(1)) {
        let done: Vec<std::io::Result<(usize, Card)>> = thread::scope(|scope| {
            let running: Vec<_> = batch
                .iter()
                .map(|(at, seed)| {
                    let (at, seed) = (*at, *seed);
                    scope.spawn(move || {
                        worth_of(tribe, &crowd[at], rung, seed).map(|worth| (at, worth))
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
        for outcome in done {
            match outcome {
                Ok((at, worth)) => marks[at].add(&worth),
                Err(wrong) => {
                    failed += 1;
                    last_words = Some(wrong);
                }
            }
        }
    }
    if failed == jobs.len()
        && let Some(wrong) = last_words
    {
        return Err(wrong);
    }
    let over = seeds.len().max(1);
    Ok((marks.iter().map(|total| total.over(over)).collect(), failed))
}

/// The crowd in the order they placed at one lesson, best first.
///
/// Judged on that lesson's marks alone. Ties are broken by where a model
/// already stood, so that two models worth the same never swap places and a
/// run stays repeatable.
pub fn placings(cards: &[Card], lesson: Lesson) -> Vec<usize> {
    let mut order: Vec<usize> = (0..cards.len()).collect();
    order.sort_by(|one, other| {
        cards[*other]
            .of(lesson)
            .partial_cmp(&cards[*one].of(lesson))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(one.cmp(other))
    });
    order
}

/// The next crowd: those that placed, and children of them to fill it out.
///
/// Children are handed round the survivors in turn rather than heaped on the
/// best one, so that a crowd does not become one model and its copies before
/// a lesson has finished asking anything of it.
pub fn next_crowd(tribe: &Tribe, crowd: &[Body], placed: &[usize], life: u32) -> Vec<Body> {
    let keep = tribe.keep.clamp(1, crowd.len());
    let mut next: Vec<Body> = placed[..keep].iter().map(|at| crowd[*at].clone()).collect();
    for at in keep..tribe.folk {
        let parent = &next[(at - keep) % keep];
        let mut dice = Dice::from_seed(
            tribe
                .seed
                .wrapping_mul(0x2545_f491)
                .wrapping_add(u64::from(life).wrapping_mul(7919))
                .wrapping_add(at as u64),
        );
        let child = parent
            .iter()
            .map(|number| number + tribe.spread * dice.spread())
            .collect();
        next.push(child);
    }
    next
}

/// What one generation came to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Life {
    /// Which generation.
    pub number: u32,
    /// What the best of the crowd was worth on the generation's own matches:
    /// the lesson's marks under mirror, the margin under swiss.
    pub best: f32,
    /// The same, over the whole crowd.
    pub middling: f32,
    /// How far this generation's children are moved off their parents.
    pub spread: f32,
    /// Matches of it that never finished.
    pub failed: usize,
}

/// How much the spread widens or narrows on the fifth rule's verdict.
pub const WIDENED_BY: f32 = 1.2;

/// How far the spread may wander from where the plan set it, either way.
///
/// The rule's verdict is a coin flip whenever children and parents are worth
/// about the same — which is true both when the spread is tiny and when it
/// is huge — and a coin-flipped multiplicative step is a drifting walk with
/// nowhere it settles. The band is what keeps a long stage inside the range
/// the plan's own number names.
pub const SPREAD_BAND: f32 = 8.0;

/// The spread after one generation's verdict, by the fifth rule.
///
/// A child sits at `keep` or beyond and its parent is the survivor at
/// `(at - keep) % keep`, which is how [`next_crowd`] lays a crowd out. Over
/// a fifth of the children beating their parents widens the spread by
/// [`WIDENED_BY`], under a fifth narrows it by the same, exactly a fifth —
/// or a crowd with no children — leaves it alone. Whatever the verdicts add
/// up to, the spread stays within [`SPREAD_BAND`] of `written`, the number
/// the plan gave.
pub fn adapted_spread(spread: f32, written: f32, worths: &[f32], keep: usize) -> f32 {
    let children = worths.len().saturating_sub(keep);
    if children == 0 {
        return spread;
    }
    let won = (keep..worths.len())
        .filter(|at| worths[*at] > worths[(at - keep) % keep])
        .count();
    let fifth = children as f32 / 5.0;
    let moved = if won as f32 > fifth {
        spread * WIDENED_BY
    } else if (won as f32) < fifth {
        spread / WIDENED_BY
    } else {
        spread
    };
    moved.clamp(written / SPREAD_BAND, written * SPREAD_BAND)
}

/// What every model of a generation came to, and the order that puts them
/// in, best first.
fn judged(
    tribe: &Tribe,
    crowd: &[Body],
    rung: &Rung,
    life: u32,
) -> std::io::Result<(Vec<f32>, Vec<usize>, usize)> {
    match tribe.selection {
        Selection::Mirror => {
            let (cards, failed) = worth_of_all(tribe, crowd, rung, &tribe.trials_of(life))?;
            let worths = cards.iter().map(|card| card.of(rung.lesson)).collect();
            let placed = placings(&cards, rung.lesson);
            Ok((worths, placed, failed))
        }
        Selection::Swiss => {
            let (margins, failed) = swiss_margins(tribe, crowd, rung, life)?;
            let placed = standings(&margins);
            Ok((margins, placed, failed))
        }
    }
}

/// Teaches a crowd one lesson, and hands back what is left of it.
///
/// The crowd that comes back is the same size as the one that went in, best
/// first, so a lesson can be handed straight to the next.
pub fn teach_a_lesson(
    tribe: &Tribe,
    crowd: Vec<Body>,
    rung: &Rung,
    mut told: impl FnMut(Life),
) -> std::io::Result<Vec<Body>> {
    let mut crowd = crowd;
    let mut spread = tribe.spread;
    for life in 1..=tribe.lives {
        let (worths, placed, failed) = judged(tribe, &crowd, rung, life)?;
        // The first crowd arrived from outside, already reordered, so it
        // carries no parentage to read the fifth rule off.
        if life > 1 {
            spread = adapted_spread(
                spread,
                tribe.spread,
                &worths,
                tribe.keep.clamp(1, crowd.len()),
            );
        }
        told(Life {
            number: life,
            best: placed.first().map_or(0.0, |at| worths[*at]),
            middling: worths.iter().sum::<f32>() / worths.len().max(1) as f32,
            spread,
            failed,
        });
        let sway = Tribe {
            spread,
            ..tribe.clone()
        };
        crowd = next_crowd(&sway, &crowd, &placed, life);
    }
    // Placed once more on the last children, so that what is handed on is in
    // order and nothing untried is called the best.
    let (_, placed, _) = judged(tribe, &crowd, rung, tribe.lives + 1)?;
    Ok(placed.into_iter().map(|at| crowd[at].clone()).collect())
}
