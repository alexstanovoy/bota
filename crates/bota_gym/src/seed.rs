//! The seeding scheme: every random choice of a run is a pure function of one run seed.
//!
//! A run seed fans out by *stream* and *domain*: [`derive_seed`]`(base, stream, domain)` mixes
//! the three with the SplitMix64 finaliser, so seeds of different streams (games, seats) and
//! of different domains (the match, an opponent draw, a policy's sampling) never repeat one
//! another however they are combined. A game's stream is its [`GameId::key`], unique per
//! `(slot, game ordinal)` for up to [`MAX_SLOTS`] slots, so the `n`th game of slot `s` meets the
//! same world and draws in every run with the same seed, whatever ran beside it.
//!
//! Domains are the caller's: any distinct 64-bit constants (an ASCII tag reads well). The gym
//! itself uses none except [`seat_seed`]'s.
//!
//! [`paired_games`] plans evaluations that cancel the side advantage: every opponent meets the
//! evaluated bot on the same seed once from each side, so a seed's world and randomness are
//! shared by both games of the pair and only the sides differ.

use crate::GymError;

/// Upper bound on the slots of one run; also the slot factor of [`GameId::key`].
pub const MAX_SLOTS: usize = 256;
/// Upper bound on planned games of one evaluation.
pub const MAX_PLANS: usize = 1 << 20;

/// Domain of [`seat_seed`].
const SEAT_DOMAIN: u64 = 0x7374_796c_655f_6472;

/// A seed for `stream` in `domain`, derived from `base`.
pub const fn derive_seed(base: u64, stream: u64, domain: u64) -> u64 {
    let mut value = base ^ stream.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ domain;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// One step of SplitMix64: advances `state` and returns the next value.
pub const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ (mixed >> 31)
}

/// The seed of one seat of a match: what that seat's own randomness (a bot's style, its
/// sampling) starts from, distinct per seat of the same match seed.
pub const fn seat_seed(match_seed: u64, seat: usize) -> u64 {
    let mut state = match_seed ^ (seat as u64).wrapping_add(1).wrapping_mul(SEAT_DOMAIN);
    splitmix64(&mut state)
}

/// A small counter-based generator: SplitMix64 with a draw count, so its state can be saved
/// and restored exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
    draws: u64,
}

/// Rejection rounds [`SplitMix64::below`] tries; each fails with probability below
/// `bound / 2^64`, so running out means the generator is broken, not unlucky.
const MAX_REJECTIONS: u32 = 64;

impl SplitMix64 {
    pub const fn new(seed: u64) -> Self {
        Self {
            state: seed,
            draws: 0,
        }
    }

    /// A generator restored from [`Self::checkpoint`].
    pub const fn from_checkpoint(state: u64, draws: u64) -> Self {
        Self { state, draws }
    }

    /// The state and the draws taken so far.
    pub const fn checkpoint(&self) -> (u64, u64) {
        (self.state, self.draws)
    }

    pub fn next_u64(&mut self) -> Result<u64, GymError> {
        self.draws = self.draws.checked_add(1).ok_or(GymError::CounterOverflow)?;
        Ok(splitmix64(&mut self.state))
    }

    /// An unbiased integer in `0..bound`.
    pub fn below(&mut self, bound: u64) -> Result<u64, GymError> {
        if bound == 0 {
            return Err(GymError::Config("random bound must be positive"));
        }
        let zone = ((1u128 << 64) / u128::from(bound)) * u128::from(bound);
        for _ in 0..MAX_REJECTIONS {
            let value = u128::from(self.next_u64()?);
            if value < zone {
                return Ok((value % u128::from(bound)) as u64);
            }
        }
        panic!("SplitMix64 rejected {MAX_REJECTIONS} draws below {bound}: the generator is broken");
    }
}

/// Which game of which slot: slot `slot` plays games `0, 1, 2, …` one after another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameId {
    pub slot: usize,
    pub game: u64,
}

impl GameId {
    /// The game's stream for [`derive_seed`]: `game * MAX_SLOTS + slot`, unique per game.
    pub fn key(self) -> Result<u64, GymError> {
        if self.slot >= MAX_SLOTS {
            return Err(GymError::Config("slot must be below MAX_SLOTS"));
        }
        self.game
            .checked_mul(MAX_SLOTS as u64)
            .and_then(|key| key.checked_add(self.slot as u64))
            .ok_or(GymError::CounterOverflow)
    }

    /// The seed of this game in `domain`, from the run seed `base`.
    pub fn seed(self, base: u64, domain: u64) -> Result<u64, GymError> {
        Ok(derive_seed(base, self.key()?, domain))
    }

    /// The slot's next game.
    pub fn next(self) -> Result<Self, GymError> {
        Ok(Self {
            slot: self.slot,
            game: self.game.checked_add(1).ok_or(GymError::CounterOverflow)?,
        })
    }
}

/// One planned game of a paired evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pairing {
    pub seed: u64,
    /// The opponent's index in the caller's pool.
    pub opponent: usize,
    /// The seat the evaluated bot plays; the opponent plays the other of a two-seat match.
    pub seat: usize,
}

/// Every opponent against the evaluated bot on both sides of every seed in
/// `first_seed..first_seed + seeds`: seed by seed, opponent by opponent, seat 0 then seat 1.
pub fn paired_games(
    first_seed: u64,
    seeds: u64,
    opponents: usize,
) -> Result<impl ExactSizeIterator<Item = Pairing>, GymError> {
    if opponents == 0 || seeds == 0 {
        return Err(GymError::Config("paired games need seeds and opponents"));
    }
    first_seed
        .checked_add(seeds)
        .ok_or(GymError::Config("paired seed range overflows"))?;
    let per_seed = 2 * opponents as u64;
    let games = seeds
        .checked_mul(per_seed)
        .filter(|games| *games <= MAX_PLANS as u64)
        .ok_or(GymError::Config("paired games exceed MAX_PLANS"))?;
    Ok((0..games as usize).map(move |index| {
        let index = index as u64;
        Pairing {
            seed: first_seed + index / per_seed,
            opponent: (index % per_seed / 2) as usize,
            seat: (index % 2) as usize,
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scheme is a compatibility contract: recorded runs replay only while these hold.
    #[test]
    fn seeds_are_pinned() {
        assert_eq!(derive_seed(1, 2, 3), 0xba56_9499_15dc_f9e9);
        assert_eq!(seat_seed(7, 1), 0xe2a5_9161_b8e1_a627);
        let mut random = SplitMix64::new(42);
        assert_eq!(random.next_u64(), Ok(0xbdd7_3226_2feb_6e95));
        assert_eq!(random.checkpoint().1, 1);
    }

    #[test]
    fn below_stays_below_its_bound_and_refuses_zero() {
        let mut random = SplitMix64::new(9);
        for bound in [1, 2, 3, 1_000_000_007, u64::MAX] {
            assert!(random.below(bound).expect("a draw") < bound);
        }
        assert_eq!(
            random.below(0),
            Err(GymError::Config("random bound must be positive"))
        );
    }

    #[test]
    fn game_keys_are_unique_per_slot_and_game_and_bounded() {
        let a = GameId { slot: 3, game: 1 }.key().expect("a key");
        let b = GameId { slot: 4, game: 1 }.key().expect("a key");
        let c = GameId { slot: 3, game: 2 }.key().expect("a key");
        assert_eq!((a, b, c), (259, 260, 515));
        assert_eq!(
            GameId {
                slot: MAX_SLOTS,
                game: 0
            }
            .key(),
            Err(GymError::Config("slot must be below MAX_SLOTS"))
        );
        assert_eq!(
            GameId {
                slot: 0,
                game: u64::MAX
            }
            .next(),
            Err(GymError::CounterOverflow)
        );
    }

    #[test]
    fn paired_games_meet_every_opponent_from_both_sides_of_every_seed() {
        let games: Vec<_> = paired_games(10, 2, 2).expect("a plan").collect();
        let expected: Vec<_> = [
            (10, 0, 0),
            (10, 0, 1),
            (10, 1, 0),
            (10, 1, 1),
            (11, 0, 0),
            (11, 0, 1),
            (11, 1, 0),
            (11, 1, 1),
        ]
        .into_iter()
        .map(|(seed, opponent, seat)| Pairing {
            seed,
            opponent,
            seat,
        })
        .collect();
        assert_eq!(games, expected);
        assert!(paired_games(u64::MAX, 1, 1).is_err());
        assert!(paired_games(0, MAX_PLANS as u64, 1).is_err());
    }
}
