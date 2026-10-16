//! Who an entity is set on.

use crate::game::Entity;

/// Who an entity is set on. Absent when it is set on nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target(pub Entity);
