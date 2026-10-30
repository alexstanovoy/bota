//! Health and mana.

use bota_proto::Fixed;

use crate::engine::state_hash;

/// Health an entity has left, held finer than a whole point. The maximum
/// is in [`Stats`].
///
/// [`Stats`]: crate::game::Stats
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Health {
    /// Health now. At or below zero the entity is dead.
    pub hp: Fixed,
}

/// Mana an entity has left, held the same way as [`Health`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mana {
    pub mana: Fixed,
}

state_hash!(Health { hp });
state_hash!(Mana { mana });
