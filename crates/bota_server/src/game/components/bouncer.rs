use bota_proto::AbilityId;

use crate::engine::{Entity, state_hash};

/// A Snowball Shot on its way from one unit to the next.
///
/// It carries a [`Transform`] for the spot of the unit it hit last and a team of its own; whoever
/// cast it may be gone by the time it has run out.
///
/// [`Transform`]: crate::game::Transform
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bouncer {
    /// Who cast it. The handle may outlive the body.
    pub source: Entity,
    /// The spell it is.
    pub ability: AbilityId,
    /// The level the spell was cast at.
    pub level: u8,
    /// The unit it hit last, which it does not bounce back to.
    pub last: Entity,
    /// Bounces it has yet to make.
    pub left: u8,
    /// The tick of its next bounce.
    pub due: u32,
}

state_hash!(Bouncer {
    source,
    ability,
    level,
    last,
    left,
    due
});
