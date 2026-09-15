//! Who an entity is set on.

use crate::game::Entity;

/// Who an entity is set on.
///
/// Absent when it is set on nobody. Who puts it here is target acquisition,
/// the lane creep mind, or an order; the attack cycle only reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target(pub Entity);
