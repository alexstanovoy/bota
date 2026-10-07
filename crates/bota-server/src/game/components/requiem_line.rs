//! One line of a requiem in flight.

use bota_proto::{Fixed, Vec2};

use crate::engine::Entity;

/// A soul let go by a requiem, flying straight out of its caster and
/// burning everything hostile it crosses, each once.
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
    /// Outgoing magical amplification captured when the line was let go.
    pub damage_amp_bp: i32,
    /// Which level of the requiem let it go, counted from zero.
    pub level: u8,
    /// Whether it frightens, slows and tears the magic resistance of each it
    /// crosses. A requiem let go on its caster's death only burns.
    pub frightens: bool,
    /// What it has crossed already, never crossed twice.
    pub struck: Vec<Entity>,
}
