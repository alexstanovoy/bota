//! A Blood Grenade in flight, and what each side's shop has left of an item
//! sold from a limited stock.

use bota_proto::{ItemId, Team, Vec2};

use crate::engine::Entity;

/// A Blood Grenade on its way to the spot it was thrown at.
///
/// It carries a [`Transform`] for where it is and a team of its own; whoever
/// threw it may be gone by the time it lands.
///
/// [`Transform`]: crate::game::Transform
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grenade {
    pub owner: Entity,
    /// Where it lands.
    pub aim: Vec2,
    /// Outgoing magical amplification captured when it was thrown.
    pub damage_amp_bp: i32,
}

/// What one side's shop has left of one item sold from a limited stock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShopStock {
    pub team: Team,
    pub item: ItemId,
    /// How many may be bought right now.
    pub left: u8,
    /// Ticks until it gains one more. Zero while it holds its most.
    pub restock_left: u32,
}
