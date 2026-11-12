//! Opponent pools: which opponent each game meets.
//!
//! A [`Mixture`] holds integer weights per opponent and draws one per game from that game's
//! seed, so a run redraws every game identically. [`Outcomes`] keeps the recent results
//! against each opponent, and [`schedule`] turns configured weights and those results into
//! the next mixture: [`Schedule::Fixed`] keeps the configured weights (equal weights draw
//! uniformly), [`Schedule::Prioritized`] favours the opponents the bot does worst against
//! (prioritised fictitious self-play). Everything is integer arithmetic, so a schedule is a
//! pure function of the recorded outcomes on every machine.
//!
//! Opponents are named by the caller's key `K` (an index into its pool, an enum of scripted
//! bots and snapshots, …); the gym never learns what they are.

use std::collections::VecDeque;

use crate::{GymError, SplitMix64};

/// Upper bound on the entries of one mixture.
pub const MAX_MIXTURE_ENTRIES: usize = 256;
/// Upper bound on one entry's weight; with [`MAX_MIXTURE_ENTRIES`] the total fits easily.
pub const MAX_WEIGHT: u64 = 1_000_000_000;
/// Upper bound on the games one opponent's window keeps.
pub const MAX_WINDOW: usize = 4096;
/// Scores are in millionths of a game.
pub const SCORE_UNIT: u64 = 1_000_000;

/// Opponents with positive integer weights, in the caller's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mixture<K> {
    entries: Vec<(K, u64)>,
    total: u64,
}

impl<K: Copy + Eq> Mixture<K> {
    /// A mixture of `1..=MAX_MIXTURE_ENTRIES` distinct opponents, each weighing `1..=MAX_WEIGHT`.
    pub fn new(entries: Vec<(K, u64)>) -> Result<Self, GymError> {
        if entries.is_empty() || entries.len() > MAX_MIXTURE_ENTRIES {
            return Err(GymError::Config(
                "a mixture needs 1..=MAX_MIXTURE_ENTRIES entries",
            ));
        }
        let mut total = 0u64;
        for (index, &(kind, weight)) in entries.iter().enumerate() {
            if !(1..=MAX_WEIGHT).contains(&weight) {
                return Err(GymError::Config("a mixture weight must be 1..=MAX_WEIGHT"));
            }
            if entries[..index].iter().any(|(other, _)| *other == kind) {
                return Err(GymError::Config("a mixture names an opponent twice"));
            }
            total += weight;
        }
        Ok(Self { entries, total })
    }

    pub fn entries(&self) -> &[(K, u64)] {
        &self.entries
    }

    pub const fn total(&self) -> u64 {
        self.total
    }

    /// The opponent of the game whose draw seed is `seed`: one weighted draw, a pure function of
    /// the seed (derive it per game, e.g. with [`GameId::seed`](crate::GameId::seed)).
    pub fn draw(&self, seed: u64) -> Result<K, GymError> {
        let mut ticket = SplitMix64::new(seed).below(self.total)?;
        for &(kind, weight) in &self.entries {
            if ticket < weight {
                return Ok(kind);
            }
            ticket -= weight;
        }
        unreachable!("the ticket is below the mixture total")
    }
}

/// How one game ended for the bot being scheduled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Score {
    Loss,
    /// A draw, or a game cut at its time limit.
    Draw,
    Win,
}

/// The bot's recent record against one opponent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub games: u64,
    pub wins: u64,
    pub draws: u64,
}

impl Record {
    /// The record's score in millionths of a game: a win is one game, a draw `draw_score`.
    pub fn score_units(self, draw_score: u64) -> u64 {
        assert!(draw_score <= SCORE_UNIT);
        assert!(self.wins + self.draws <= self.games);
        self.wins * SCORE_UNIT + self.draws * draw_score
    }

    pub fn add(&mut self, other: Self) {
        self.games += other.games;
        self.wins += other.wins;
        self.draws += other.draws;
    }
}

/// The last `window` results against each opponent, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcomes<K> {
    window: usize,
    max_opponents: usize,
    entries: Vec<(K, VecDeque<Score>)>,
}

impl<K: Copy + Eq> Outcomes<K> {
    /// Keeps `1..=MAX_WINDOW` results for each of at most `max_opponents` opponents.
    pub fn new(window: usize, max_opponents: usize) -> Result<Self, GymError> {
        if !(1..=MAX_WINDOW).contains(&window) {
            return Err(GymError::Config("an outcome window must be 1..=MAX_WINDOW"));
        }
        if max_opponents == 0 {
            return Err(GymError::Config("outcomes need room for an opponent"));
        }
        Ok(Self {
            window,
            max_opponents,
            entries: Vec::new(),
        })
    }

    /// Books one finished game, forgetting the oldest beyond the window.
    pub fn record(&mut self, opponent: K, score: Score) -> Result<(), GymError> {
        let index = match self.entries.iter().position(|(kind, _)| *kind == opponent) {
            Some(index) => index,
            None => {
                if self.entries.len() >= self.max_opponents {
                    return Err(GymError::Config("outcomes hold max_opponents opponents"));
                }
                self.entries
                    .push((opponent, VecDeque::with_capacity(self.window)));
                self.entries.len() - 1
            }
        };
        let scores = &mut self.entries[index].1;
        if scores.len() == self.window {
            scores.pop_front();
        }
        scores.push_back(score);
        Ok(())
    }

    /// Games, wins and draws in the window against `opponent`.
    pub fn tally(&self, opponent: K) -> Record {
        self.entries
            .iter()
            .find(|(kind, _)| *kind == opponent)
            .map_or_else(Record::default, |(_, scores)| {
                let count = |wanted: Score| scores.iter().filter(|&&got| got == wanted).count();
                Record {
                    games: scores.len() as u64,
                    wins: count(Score::Win) as u64,
                    draws: count(Score::Draw) as u64,
                }
            })
    }

    /// Forgets opponents that no longer play.
    pub fn retain(&mut self, playing: impl Fn(K) -> bool) {
        self.entries.retain(|(kind, _)| playing(*kind));
    }

    /// Every opponent's window, in first-recorded order: what a checkpoint saves.
    pub fn entries(&self) -> impl Iterator<Item = (K, &VecDeque<Score>)> {
        self.entries.iter().map(|(kind, scores)| (*kind, scores))
    }

    /// Outcomes restored from [`Self::entries`]; refuses what [`Self::new`] and
    /// [`Self::record`] could not have produced.
    pub fn restore(
        window: usize,
        max_opponents: usize,
        entries: Vec<(K, VecDeque<Score>)>,
    ) -> Result<Self, GymError> {
        let mut outcomes = Self::new(window, max_opponents)?;
        for (index, (kind, scores)) in entries.iter().enumerate() {
            if scores.len() > window || entries[..index].iter().any(|(other, _)| other == kind) {
                return Err(GymError::Config("restored outcomes are malformed"));
            }
        }
        if entries.len() > max_opponents {
            return Err(GymError::Config("outcomes hold max_opponents opponents"));
        }
        outcomes.entries = entries;
        Ok(outcomes)
    }
}

/// How configured weights become a mixture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schedule {
    /// The configured weights unchanged.
    Fixed,
    /// Each weight times `max((1 - p)², 1/floor_divisor)`, where `p` is the Laplace-smoothed
    /// score against that opponent and a draw scores `draw_score` millionths of a win.
    Prioritized { draw_score: u64, floor_divisor: u64 },
}

/// `weight * max((1 - p)^2, 1/floor_divisor)` with `p = (score + 1) / (games + 2)`, a draw
/// scoring `draw_score` millionths of a win; at least one unit, so every opponent stays drawable.
pub fn prioritized_weight(weight: u64, record: Record, draw_score: u64, floor_divisor: u64) -> u64 {
    assert!(floor_divisor > 0);
    let unit = u128::from(SCORE_UNIT);
    let score = u128::from(record.score_units(draw_score));
    let numerator = u128::from(record.games + 1) * unit - score;
    let denominator = u128::from(record.games + 2) * unit;
    let scaled = u128::from(weight) * numerator * numerator / (denominator * denominator);
    u64::try_from(scaled)
        .expect("never above the configured weight")
        .max(weight / floor_divisor)
        .max(1)
}

/// The mixture `schedule` makes of the configured `entries` and the recorded `outcomes`.
pub fn schedule<K: Copy + Eq>(
    entries: &[(K, u64)],
    outcomes: &Outcomes<K>,
    schedule: Schedule,
) -> Result<Mixture<K>, GymError> {
    if let Schedule::Prioritized {
        draw_score,
        floor_divisor,
    } = schedule
        && (draw_score > SCORE_UNIT || floor_divisor == 0)
    {
        return Err(GymError::Config(
            "a draw scores 0..=SCORE_UNIT and the floor divisor is positive",
        ));
    }
    Mixture::new(
        entries
            .iter()
            .map(|&(kind, weight)| {
                let weight = match schedule {
                    Schedule::Fixed => weight,
                    Schedule::Prioritized {
                        draw_score,
                        floor_divisor,
                    } => {
                        prioritized_weight(weight, outcomes.tally(kind), draw_score, floor_divisor)
                    }
                };
                (kind, weight)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draw_is_a_pure_function_of_its_seed_and_follows_the_weights() {
        let mixture = Mixture::new(vec![('a', 1), ('b', 3)]).expect("a mixture");
        let draws: Vec<_> = (0..4000)
            .map(|seed| mixture.draw(seed).expect("a draw"))
            .collect();
        let again: Vec<_> = (0..4000)
            .map(|seed| mixture.draw(seed).expect("a draw"))
            .collect();
        assert_eq!(draws, again);
        let b = draws.iter().filter(|&&kind| kind == 'b').count();
        assert!((2800..3200).contains(&b), "{b} of 4000 draws were b");
    }

    #[test]
    fn mixtures_refuse_empty_zero_weight_and_duplicate_entries() {
        assert!(Mixture::<u8>::new(Vec::new()).is_err());
        assert_eq!(
            Mixture::new(vec![(1, 0)]),
            Err(GymError::Config("a mixture weight must be 1..=MAX_WEIGHT"))
        );
        assert_eq!(
            Mixture::new(vec![(1, 1), (1, 2)]),
            Err(GymError::Config("a mixture names an opponent twice"))
        );
    }

    #[test]
    fn prioritized_weights_favour_unbeaten_opponents_and_keep_a_floor() {
        let record = |games, wins| Record {
            games,
            wins,
            draws: 0,
        };
        // No games: p = 1/2, weight * 1/4.
        assert_eq!(prioritized_weight(1_000, record(0, 0), 0, 10), 250);
        // Always lost: p = 1/102, (101/102)^2 of the weight.
        assert_eq!(prioritized_weight(1_000, record(100, 0), 0, 10), 980);
        // Always won: the floor of a tenth.
        assert_eq!(prioritized_weight(1_000, record(100, 100), 0, 10), 100);
        // Never zero.
        assert_eq!(prioritized_weight(1, record(100, 100), 0, 10), 1);
        // A draw scoring a full win counts as one.
        let draws = Record {
            games: 100,
            wins: 0,
            draws: 100,
        };
        assert_eq!(prioritized_weight(1_000, draws, SCORE_UNIT, 10), 100);
    }

    #[test]
    fn outcomes_keep_the_last_window_and_restore_what_they_saved() {
        let mut outcomes = Outcomes::new(3, 2).expect("outcomes");
        for score in [Score::Win, Score::Win, Score::Loss, Score::Draw] {
            outcomes.record('a', score).expect("recorded");
        }
        assert_eq!(
            outcomes.tally('a'),
            Record {
                games: 3,
                wins: 1,
                draws: 1
            }
        );
        outcomes.record('b', Score::Win).expect("recorded");
        assert_eq!(
            outcomes.record('c', Score::Win),
            Err(GymError::Config("outcomes hold max_opponents opponents"))
        );
        let saved: Vec<_> = outcomes
            .entries()
            .map(|(kind, scores)| (kind, scores.clone()))
            .collect();
        assert_eq!(Outcomes::restore(3, 2, saved), Ok(outcomes));
    }
}
