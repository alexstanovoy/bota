//! What Pudge's dismember is worth, as pure functions of the numbers a player
//! may know.

use bota_proto::Fixed;

use crate::{Dec, Ms, fixed, rules};

/// Damage one bite of a dismember deals before resistance: the channel's
/// damage a second, which its caster's strength adds to by `strength_factor`
/// a point, over `channel` shared out over `bites` bites.
#[inline]
pub fn dismember_bite_damage(
    per_second: Dec,
    strength_factor: Dec,
    strength: Fixed,
    channel: Ms,
    bites: u32,
) -> i32 {
    let per_second = fixed(per_second) + strength * fixed(strength_factor);
    (per_second * Fixed::from_ratio(channel.0 as i32, 1000 * bites as i32)).to_int()
}

/// Ticks after a dismember takes hold at which its `bite`th bite of `bites`
/// lands, from zero for the one as it takes hold: `channel` cut evenly, each
/// beat rounded up to a whole tick.
#[inline]
pub fn dismember_bite_tick(bite: u32, bites: u32, channel: Ms) -> u32 {
    let channel_ticks_x1000 = channel.0 * rules::TICKS_PER_SECOND;
    (bite * channel_ticks_x1000).div_ceil(1000 * bites)
}
