//! What Pudge's dismember is worth, as pure functions of the numbers a player
//! may know.

use bota_proto::Fixed;

use crate::rules;

/// Milliseconds of the dismember's full channel: 2.75 seconds.
const CHANNEL_MS: i32 = 2750;

/// Damage one bite of a dismember deals before resistance: the channel's
/// damage a second, which its caster's strength adds to, shared out over
/// [`rules::DISMEMBER_BITES`] bites. `level` counts from zero.
#[inline]
pub fn dismember_bite_damage(level: usize, strength: Fixed) -> i32 {
    let per_second = Fixed::from_int(rules::DISMEMBER_DAMAGE_PER_SECOND[level])
        + strength * rules::DISMEMBER_STRENGTH_DAMAGE[level];
    (per_second * Fixed::from_ratio(CHANNEL_MS, 1000 * rules::DISMEMBER_BITES as i32)).to_int()
}

/// Ticks after a dismember takes hold at which its `bite`th bite lands, from
/// zero for the one as it takes hold: the channel cut evenly, each beat
/// rounded up to a whole tick.
#[inline]
pub fn dismember_bite_tick(bite: u32) -> u32 {
    let channel_ticks_x1000 = CHANNEL_MS as u32 * rules::TICKS_PER_SECOND;
    (bite * channel_ticks_x1000).div_ceil(1000 * rules::DISMEMBER_BITES)
}
