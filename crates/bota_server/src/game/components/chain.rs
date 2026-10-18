use bota_proto::{Fixed, Vec2};

use crate::engine::Entity;

/// A chain of damage leaping from one enemy to the next, a beat apart.
///
/// It carries a [`Transform`] for the spot its last leap landed and a team of
/// its own; whoever cast it may be gone by the time it has run out.
///
/// [`Transform`]: crate::game::Transform
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    /// Who cast it. The handle may outlive the body.
    pub source: Entity,
    /// Where the last leap landed, kept for when its target has fallen.
    pub at: Vec2,
    /// Whom it has struck already, never struck twice.
    pub struck: Vec<Entity>,
    /// Leaps it may still make.
    pub leaps_left: u8,
    /// Damage the next leap deals.
    pub damage: i32,
    /// Percent of the damage the leap after keeps.
    pub keep_pct: i32,
    /// How far a leap reaches from the last spot.
    pub radius: Fixed,
    /// Ticks between two leaps.
    pub delay: u32,
    /// Ticks before the next leap.
    pub wait: u32,
}
