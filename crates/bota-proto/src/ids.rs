//! Identifiers and small enumerations used across every message.

use serde::{Deserialize, Serialize};
/// A handle to an entity in the world: a unit, a projectile or an item on the
/// ground.
///
/// Generational: a handle to a dead entity never becomes valid again.
/// Meaningful only within the match that issued it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId {
    /// Slot index into the entity arena.
    pub idx: u32,
    /// Generation counter, bumped every time the slot is reused.
    pub generation: u32,
}

/// A seat in the match, `0..` the number of seats.
///
/// Fixed for the whole match, so it is usable as an array index.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotId(pub u8);

/// A connection, as seen by the network layer.
///
/// Every connection gets a fresh one. A spectator has a `PlayerId` and no
/// [`SlotId`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u32);

/// A side of the map, or nobody's side.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Team {
    /// The side spawning in the lower left corner.
    Radiant,
    /// The side spawning in the upper right corner.
    Dire,
    /// The jungle's own: hostile to both sides. No seat plays for it.
    Neutral,
}

/// Selects one of the playable heroes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeroId(pub u16);

/// Selects one specific ability, independent of which hero owns it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbilityId(pub u16);

/// Selects one purchasable item.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(pub u16);

/// How something in a slot is aimed when it is used.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Aim {
    /// At nothing: it works on whoever used it.
    Own,
    /// At a spot on the ground.
    Point,
    /// At a unit.
    Unit,
    /// At the tree standing on the spot it is pointed at.
    Tree,
    /// At a spot within reach of an allied tower, Ancient or fountain.
    Building,
}

/// Names one kind of timed effect a unit can be under.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectId(pub u16);

/// An index into a unit's ability slots, as listed in
/// [`UnitView::abilities`](crate::UnitView::abilities).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbilitySlot(pub u8);

/// One of a hero's fifteen item slots.
///
/// Slots 0-5 are the inventory, where items work; 6-8 the backpack, where
/// they are carried inert; 9-14 the stash waiting at the home shop.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemSlot(pub u8);

/// Selects the terrain and building layout a match is played on.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapId(pub u16);

/// What kind of thing a unit in a view is.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnitKind {
    /// A hero controlled by a player or a bot.
    Hero,
    /// A melee lane creep.
    CreepMelee,
    /// A melee lane creep carrying the flag: resistant to magic, and mends
    /// the health of its own side nearby.
    CreepFlagbearer,
    /// A ranged lane creep.
    CreepRanged,
    /// A siege creep.
    CreepSiege,
    /// A neutral camp creep.
    CreepNeutral,
    /// A lane tower.
    Tower,
    /// The structure that ends the match when destroyed.
    Ancient,
    /// A barracks; while it stands, the enemy creeps of its kind in its lane
    /// stay plain.
    Barracks,
    /// The fountain, which mends its own side and attacks enemies in reach.
    Fountain,
    /// An observer ward placed by a hero.
    Ward,
    /// A courier, one to a seat, that carries what its owner bought.
    Courier,
}
