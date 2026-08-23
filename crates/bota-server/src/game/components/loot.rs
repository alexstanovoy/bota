//! An item lying on the ground.

use crate::game::ItemStack;

/// The stack an entity that is a ground item holds.
///
/// The stack rides whole: charges, attribute mode, mute and ownership come
/// back up exactly as they went down. The entity has no team, no health and
/// no hull; it is walked through and lies there until somebody takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loot(pub ItemStack);
