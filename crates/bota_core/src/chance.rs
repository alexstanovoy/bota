//! Declared distributions: what a rule promises on average, never how a
//! single draw falls. The draws themselves stay with the server.

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

/// An exact rate: `Ratio::new(3, 10)` hits three times in every ten
/// attempts, not three times on average.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ratio {
    num: u8,
    den: u8,
}

impl Ratio {
    /// Largest denominator a rate may have: the width of the 64-bit mask the
    /// server deals a block of attempts from.
    pub const MAX_DEN: u8 = 64;

    /// Never hits.
    pub const NEVER: Ratio = Ratio { num: 0, den: 1 };

    /// `num` hits out of every `den` attempts.
    ///
    /// Panics unless `0 < den <= MAX_DEN` and `num <= den`.
    #[inline]
    pub const fn new(num: u8, den: u8) -> Ratio {
        assert!(den > 0, "a ratio needs a denominator");
        assert!(den <= Ratio::MAX_DEN, "denominator above MAX_DEN");
        assert!(num <= den, "more hits than attempts");
        Ratio { num, den }
    }

    /// Hits per block.
    #[inline]
    pub const fn num(self) -> u8 {
        self.num
    }

    /// Attempts per block.
    #[inline]
    pub const fn den(self) -> u8 {
        self.den
    }

    /// Whether it hits more often than another.
    #[inline]
    pub const fn beats(self, other: Ratio) -> bool {
        (self.num as u16) * (other.den as u16) > (other.num as u16) * (self.den as u16)
    }
}

impl Default for Ratio {
    #[inline]
    fn default() -> Ratio {
        Ratio::NEVER
    }
}
