//! An item lying on the ground.

use crate::game::ItemStack;

/// The stack an entity that is a ground item holds, whole but marked
/// touched. The entity has no team, no health and no hull, and lies there
/// until somebody takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loot(pub ItemStack);
