//! A hook in flight.

use bota_proto::{Fixed, Vec2};

use crate::engine::Entity;
use crate::game::rules;

/// A hook thrown out and on its way somewhere.
///
/// It flies out until it catches something or runs out of reach, then comes
/// back to whoever threw it, dragging what it caught.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hook {
    /// Who threw it.
    pub owner: Entity,
    /// The spot it flies out at.
    pub aim: Vec2,
    /// World units a second.
    pub speed: Fixed,
    /// How far it may still fly out.
    pub reach_left: Fixed,
    /// How wide it catches.
    pub radius: Fixed,
    /// Damage it deals to an enemy it catches.
    pub damage: i32,
    /// What it caught, while it is dragging it.
    pub caught: Option<Entity>,
    /// Whether it is on its way back.
    pub returning: bool,
    /// The marks its chain is laid out with, evenly between the thrower and
    /// the hook.
    pub links: [Entity; rules::HOOK_LINKS],
}
