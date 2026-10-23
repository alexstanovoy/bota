//! What makes a unit an illusion of a hero.

use crate::game::Entity;

/// What makes an entity an illusion: the hero it is an illusion of, and when
/// it was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Illusion {
    /// The hero it shows as.
    pub of: Entity,
    /// The tick it was made on.
    pub made: u32,
}
