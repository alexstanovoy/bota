//! What an entity has been told to do.

use bota_proto::Vec2;

use crate::game::Entity;

/// The standing order an entity is following.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitOrder {
    /// Stand still. Takes on enemies that come near of its own accord.
    Idle,
    /// Stand still and take on nothing at all.
    ///
    /// What a stop order leaves behind: the entity keeps the ground it is on
    /// and pays no attention to whoever walks past.
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

/// The order in hand, and when the next one may re-aim it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orders {
    /// What it is doing.
    pub current: UnitOrder,
    /// Ticks before an attack order may re-aim it. Zero when it answers the
    /// next one.
    pub cooldown: u32,
}
