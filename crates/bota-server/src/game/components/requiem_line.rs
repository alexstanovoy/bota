//! One line of a requiem in flight.

use bota_proto::{Fixed, Vec2};

use crate::engine::Entity;

/// A soul let go by a requiem, flying straight out of its caster and
/// burning everything hostile it crosses, each once, with fear and a slow.
///
/// It carries a [`Transform`] for where it is and a team of its own; whoever
/// let it go may be gone by the time it has flown out.
///
/// [`Transform`]: crate::game::Transform
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiemLine {
    /// Who let it go.
    pub owner: Entity,
    /// The spot it flies out to.
    pub aim: Vec2,
    /// World units a second.
    pub speed: Fixed,
    /// How far it has flown.
    pub travelled: Fixed,
    /// How far it flies in all.
    pub distance: Fixed,
    /// Damage it lands on each it crosses.
    pub damage: i32,
    /// Percent of speed it takes from each it crosses.
    pub slow_pct: i32,
    /// What it has crossed already, never crossed twice.
    pub struck: Vec<Entity>,
}
