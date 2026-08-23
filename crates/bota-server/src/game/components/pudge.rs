//! What Pudge carries that nothing else does.

use crate::engine::Entity;

/// The rot, left switched on.
///
/// Present while it burns; absent while it does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rotting {
    /// Which level of it is running, counted from zero.
    pub level: usize,
}

/// A dismember being channelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dismembering {
    /// What is being held.
    pub target: Entity,
    /// Ticks before it lets go.
    pub ticks_left: u32,
    /// Which level of it is running, counted from zero.
    pub level: usize,
}
