use bota_proto::AbilityId;

use crate::engine::{Entity, state_hash};

/// A spell striking in pulses at a spot, a beat apart (Miner's Slam).
///
/// It carries a [`Transform`] for the spot and a team of its own; whoever cast it may be gone by
/// the time it has run out.
///
/// [`Transform`]: crate::game::Transform
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pulser {
    /// Who cast it. The handle may outlive the body.
    pub source: Entity,
    /// The spell it is.
    pub ability: AbilityId,
    /// The level the spell was cast at.
    pub level: u8,
    /// Pulses it has yet to strike.
    pub left: u8,
    /// Ticks between two pulses.
    pub interval: u32,
    /// The tick the next pulse strikes on.
    pub due: u32,
}

state_hash!(Pulser {
    source,
    ability,
    level,
    left,
    interval,
    due
});
