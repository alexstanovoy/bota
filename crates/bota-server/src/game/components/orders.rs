//! What an entity has been told to do.

use bota_proto::Vec2;

use crate::game::{Entity, PendingCast};

/// The standing order an entity is following.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitOrder {
    /// Stand still. Takes on enemies that come near of its own accord.
    Idle,
    /// Stand still and take on nothing at all: what a stop order leaves
    /// behind.
    Stand,
    /// Stand still, attack what comes into range, never move.
    Hold,
    /// Walk to a position, ignoring enemies.
    Move {
        /// Destination.
        pos: Vec2,
    },
    /// Walk to a position, taking on enemies met on the way.
    AttackMove {
        /// Destination.
        pos: Vec2,
    },
    /// Attack one entity, following it while it stays visible.
    Attack {
        /// The target.
        target: Entity,
        /// Where the target was last seen by this entity's side.
        last_seen: Vec2,
    },
    /// Walk after one entity, taking on nobody.
    Follow {
        /// The entity being followed.
        target: Entity,
        /// Where it was last seen by this entity's side.
        last_seen: Vec2,
    },
}

/// The order in hand, a cast waiting on it, and how soon an enemy hero's
/// attack order may draw it again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orders {
    /// What it is doing.
    pub current: UnitOrder,
    /// Ticks before an enemy hero's attack order may draw it again. Zero
    /// when it answers the next one.
    pub cooldown: u32,
    /// A cast ordered and not yet begun. Any order to the body takes it
    /// away.
    pub pending: Option<PendingCast>,
}
