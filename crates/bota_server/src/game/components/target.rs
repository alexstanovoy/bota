use crate::engine::state_hash;
use crate::game::Entity;

/// Who an entity is set on. Absent when it is set on nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target(pub Entity);

state_hash!(Target(0));
