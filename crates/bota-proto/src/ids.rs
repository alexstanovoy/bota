//! Identifiers and small enumerations used across every message.

use serde::{Deserialize, Serialize};
/// A handle to a live entity in the world: a unit, a building or a ward.
///
/// Used by orders to name a target, by views to key each unit, and by events to
/// attribute damage. Handles are generational, so a handle to a dead entity
/// never becomes valid again.
///
/// Opaque to a client, and meaningful only within the match that issued it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId {
    /// Slot index into the entity arena.
    pub idx: u32,
    /// Generation counter, bumped every time the slot is reused.
    pub generation: u32,
}

/// A seat in the match, from zero up to the number of participants.
///
/// Assigned when the lobby fills and stable for the rest of the match, so it is
/// usable as an array index and as a sort key for orders.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotId(pub u8);

/// A connected participant, as seen by the network layer.
///
/// Distinct from [`SlotId`]: a spectator has a `PlayerId` and no slot, and a
/// reconnecting player gets a fresh `PlayerId` for the same slot.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u32);

/// A side of the map, or nobody's side.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Team {
    /// The side spawning in the lower left corner.
    Radiant,
    /// The side spawning in the upper right corner.
    Dire,
    /// The jungle's own: hostile to both sides, seats never sit here.
    Neutral,
}

/// Selects one of the playable heroes.
///
/// Used in lobby picks and to name the hero behind a hero unit.
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
    /// At a spot within reach of an allied building.
    Building,
}

/// Names one kind of timed effect a unit can be under.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectId(pub u16);

/// Which of a hero's four ability slots is meant.
///
/// Slots 0 to 2 are the basic abilities and slot 3 is the ultimate.
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
    /// A melee lane creep carrying the flag: tougher against magic, and its
    /// death pays every enemy hero nearby.
    CreepFlagbearer,
    /// A ranged lane creep.
    CreepRanged,
    /// A siege creep, spawned with every fifth wave.
    CreepSiege,
    /// A neutral camp creep.
    CreepNeutral,
    /// Roshan, the boss of the river pit.
    Roshan,
    /// A lane tower.
    Tower,
    /// The structure that ends the match when destroyed.
    Ancient,
    /// The fountain, which heals its own team and burns intruders.
    Fountain,
    /// An observer ward placed by a hero.
    Ward,
    /// A courier, one to a seat, that carries what its owner bought.
    Courier,
}
