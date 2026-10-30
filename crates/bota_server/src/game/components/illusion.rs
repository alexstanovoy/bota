//! What makes a unit an illusion of a hero.

use crate::engine::state_hash;
use crate::game::Entity;

/// What makes an entity an illusion: the hero it is an illusion of, and when
/// it was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Illusion {
    /// The hero it shows as.
    pub of: Entity,
    /// The tick it was made on.
    pub made: u32,
    /// The share of its hero's damage it deals, in basis points where
    /// 10_000 is all of it.
    pub dealt_bp: i32,
    /// The share of the damage it takes that lands on it, in basis points
    /// where 10_000 is all of it.
    pub taken_bp: i32,
}

state_hash!(Illusion {
    of,
    made,
    dealt_bp,
    taken_bp
});
