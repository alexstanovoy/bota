//! Declared distributions: what a rule promises on average, never how a
//! single draw falls. The draws themselves stay with the server.

use bota_proto::Fixed;

/// The gold a kill pays: a whole number drawn evenly from `min..=max` each
/// time it is paid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoldRange {
    /// The least it pays, at least zero.
    pub min: i32,
    /// The most it pays, at least `min`.
    pub max: i32,
}

impl GoldRange {
    /// Pays nothing.
    pub const ZERO: GoldRange = GoldRange::exactly(0);

    /// Anything from `min` to `max` inclusive.
    ///
    /// Panics unless `0 <= min <= max`.
    #[inline]
    pub const fn new(min: i32, max: i32) -> GoldRange {
        assert!(min >= 0, "a bounty pays no less than nothing");
        assert!(min <= max, "a bounty range runs low to high");
        GoldRange { min, max }
    }

    /// Always the same amount.
    #[inline]
    pub const fn exactly(gold: i32) -> GoldRange {
        GoldRange::new(gold, gold)
    }

    /// The same range moved up by `gold` at both ends.
    #[inline]
    pub const fn raised(self, gold: i32) -> GoldRange {
        GoldRange::new(self.min + gold, self.max + gold)
    }
}

/// A chance, in parts of [`Odds::WHOLE`]: exact to a millionth of a percent.
///
/// Chances that stack do so as Dota stacks evasion and accuracy,
/// diminishingly ([`Odds::or`]), and a source of chance rolls with Dota's
/// pseudo-random distribution, whose step is [`Odds::prd_step`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Odds(u32);

impl Odds {
    /// The parts a certainty holds.
    pub const WHOLE: u32 = 100_000_000;

    /// Never hits.
    pub const NEVER: Odds = Odds(0);

    /// Always hits.
    pub const ALWAYS: Odds = Odds(Odds::WHOLE);

    /// `pct` out of a hundred.
    ///
    /// Panics unless `pct` is within `0..=100`.
    #[inline]
    pub const fn percent(pct: u8) -> Odds {
        assert!(pct <= 100, "a percentage is at most a hundred");
        Odds(pct as u32 * (Odds::WHOLE / 100))
    }

    /// `parts` out of [`Odds::WHOLE`].
    ///
    /// Panics above a certainty.
    #[inline]
    pub const fn parts(parts: u32) -> Odds {
        assert!(parts <= Odds::WHOLE, "no chance is above a certainty");
        Odds(parts)
    }

    /// Its parts of [`Odds::WHOLE`].
    #[inline]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// As a fraction of a certainty, the way a view tells a chance.
    #[inline]
    pub const fn fraction(self) -> Fixed {
        Fixed::from_ratio(self.0 as i32, Odds::WHOLE as i32)
    }

    /// The chance that this or another, rolled apart, hits: `1 - (1 - a)(1 -
    /// b)`, to the nearest part. Dota stacks evasion and accuracy this way,
    /// each source taking its share of what the others leave.
    #[inline]
    pub const fn or(self, other: Odds) -> Odds {
        let whole = Odds::WHOLE as u64;
        let left = (whole - self.0 as u64) * (whole - other.0 as u64);
        Odds(Odds::WHOLE - ((left + whole / 2) / whole) as u32)
    }

    /// Whether it hits more often than another.
    #[inline]
    pub const fn beats(self, other: Odds) -> bool {
        self.0 > other.0
    }

    /// The step of Dota's pseudo-random distribution for this chance, in
    /// parts of `2^32`: the `n`-th attempt since the last hit hits with `n`
    /// steps, so a run of misses grows ever likelier to end and the rate
    /// over many attempts is the chance itself. Zero for a chance that never
    /// hits, `2^32` for one that always does.
    ///
    /// Found by halving: the smallest step whose rate is the chance or more,
    /// the rate worked out exactly in fixed point.
    pub fn prd_step(self) -> u64 {
        const SCALE: u64 = 1 << 32;
        if self.0 == 0 {
            return 0;
        }
        if self.0 >= Odds::WHOLE {
            return SCALE;
        }
        let (mut low, mut high) = (1u64, SCALE);
        while low < high {
            let mid = low + (high - low) / 2;
            if prd_rate_at_least(mid, self.0) {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        low
    }
}

/// Whether the pseudo-random distribution of `step` (parts of `2^32`) hits
/// at a rate of at least `parts` of [`Odds::WHOLE`]: whether the attempts it
/// takes to hit, on average, are at most the chance's inverse.
fn prd_rate_at_least(step: u64, parts: u32) -> bool {
    const SCALE: u128 = 1 << 32;
    // Probabilities in parts of 2^64.
    let one: u128 = 1 << 64;
    let (mut survive, mut attempts, mut n) = (one, 0u128, 0u128);
    loop {
        n += 1;
        let hit = (u128::from(step) * n).min(SCALE);
        attempts += n * (survive * hit / SCALE);
        if hit == SCALE {
            break;
        }
        survive = survive * (SCALE - hit) / SCALE;
    }
    attempts * u128::from(parts) <= one * u128::from(Odds::WHOLE)
}

#[cfg(test)]
mod tests {
    use super::Odds;

    #[test]
    fn evasions_stack_as_dota_stacks_them() {
        let talisman = Odds::percent(15);
        let butterfly = Odds::percent(35);
        assert_eq!(
            talisman.or(butterfly),
            Odds::parts(44_750_000),
            "1 - 0.85 * 0.65"
        );
        assert_eq!(butterfly.or(talisman), talisman.or(butterfly));
        assert_eq!(butterfly.or(Odds::NEVER), butterfly);
        assert_eq!(butterfly.or(Odds::ALWAYS), Odds::ALWAYS);
        let three = butterfly.or(butterfly).or(butterfly);
        assert_eq!(three, Odds::parts(72_537_500), "1 - 0.65^3");
    }

    #[test]
    fn the_pseudo_random_step_is_dotas() {
        // Dota's published constants, C(p), in parts of 2^32: 0.0038016583,
        // 0.0847440919 and 0.3021030253.
        for (pct, dota) in [(5, 16_327_998), (25, 363_973_103), (50, 1_297_522_614)] {
            let step = Odds::percent(pct).prd_step();
            assert!(
                step.abs_diff(dota) <= 64,
                "{pct}%: step {step} against {dota}"
            );
        }
        // Past half a chance the second attempt is certain: 80% is three
        // quarters, then all of it.
        assert_eq!(Odds::percent(80).prd_step(), 3 << 30);
        assert_eq!(Odds::NEVER.prd_step(), 0);
        assert_eq!(Odds::ALWAYS.prd_step(), 1 << 32);
    }
}
