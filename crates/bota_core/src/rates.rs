//! What a cooldown or a mana cost comes to under a rate: the one place the simulator and a
//! client's tooltips both read.

use crate::rules::NOMINAL_BP;

/// A mana cost after a rate in basis points.
pub fn cost_after(base: i32, rate_bp: i32) -> i32 {
    (i64::from(base) * i64::from(rate_bp) / i64::from(NOMINAL_BP)).clamp(0, i64::from(i32::MAX))
        as i32
}

/// Ticks a cooldown comes to at a rate in basis points, never zero for a cooldown that was set
/// at all.
pub fn cooldown_after(base: u32, rate_bp: i32) -> u32 {
    if base == 0 {
        return 0;
    }
    (i64::from(base) * i64::from(rate_bp) / i64::from(NOMINAL_BP)).clamp(1, i64::from(u32::MAX))
        as u32
}

/// A rate with a cut of `cut_bp` basis points taken off what it comes to: cuts stack
/// multiplicatively (an Arcane Rune's [LIQ Runes]).
pub fn cut_rate(rate_bp: i32, cut_bp: i32) -> i32 {
    (i64::from(rate_bp) * i64::from(NOMINAL_BP - cut_bp) / i64::from(NOMINAL_BP)) as i32
}
