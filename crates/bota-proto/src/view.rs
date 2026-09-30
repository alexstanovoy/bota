//! What one side is allowed to know about the world right now.

use crate::{
    AbilityId, Aim, Angle, Attribute, Attributes, EffectId, EntityId, Fixed, HeroId, ItemId,
    SlotId, Team, UnitKind, Vec2,
};
use serde::{Deserialize, Serialize};

/// Conditions currently affecting a unit, as a bit set of the associated
/// constants.
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct StatusFlags {
    /// The raw bits.
    pub bits: u16,
}

impl StatusFlags {
    /// Cannot act, move or turn.
    pub const STUNNED: u16 = 1 << 0;
    /// Movement speed is reduced.
    pub const SLOWED: u16 = 1 << 4;
    /// Losing health over time.
    pub const DOT: u16 = 1 << 5;
    /// Seen by the other team only through true sight.
    pub const INVISIBLE: u16 = 1 << 6;
    /// Immune to magical damage and most disables.
    pub const MAGIC_IMMUNE: u16 = 1 << 7;
    /// Cannot be attacked or damaged at all.
    pub const INVULNERABLE: u16 = 1 << 9;
    /// Performing a channelled ability or item action.
    pub const CHANNELLING: u16 = 1 << 10;
    /// Running from whoever put the fear on; cannot attack or cast.
    pub const FEARED: u16 = 1 << 11;
}

/// One ability slot of a visible unit, enemy or friendly.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AbilityView {
    /// Which ability sits in this slot.
    pub id: AbilityId,
    /// Current level. Zero means it has not been learned yet.
    pub level: u8,
    /// How far it may be levelled.
    pub max_level: u8,
    /// Ticks remaining before it can be cast again. Zero means ready.
    pub cooldown_left: u32,
    /// Mana the next cast would cost at the current level.
    pub mana_cost: i32,
    /// How far the next cast would reach, in whole world units. Zero for one
    /// that reaches nowhere.
    pub range: i32,
    /// How a cast of it is aimed.
    pub aim: Aim,
    /// Whether it works on its own and is never cast.
    pub passive: bool,
    /// Whether it is a toggle that is currently on.
    pub on: bool,
    /// Whether a skill point could be spent on it right now.
    pub can_level: bool,
}

/// An effect currently on a visible unit.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectView {
    /// Which effect it is.
    pub id: EffectId,
    /// Ticks until it wears off. Absent for one that does not run out.
    pub ticks_left: Option<u32>,
    /// How many are held. Absent for one that is not counted.
    pub stacks: Option<u32>,
}

/// One item slot of a visible unit.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemView {
    /// Which item sits in this slot.
    pub id: ItemId,
    /// Charges left. Absent for an item that has no charges at all.
    pub charges: Option<u8>,
    /// Ticks remaining before the item can be used again. Zero means ready.
    pub cooldown_left: u32,
    /// Ticks it stays inert after coming out of the backpack. Zero means
    /// ready.
    pub mute_left: u32,
    /// Which attribute it is set to. Absent for an item that is not set to one.
    pub mode: Option<Attribute>,
    /// Mana one use would cost.
    pub mana_cost: i32,
    /// How far one use would reach, in whole world units. Zero for one that
    /// reaches nowhere of its own.
    pub range: i32,
    /// How a use of it is aimed. Absent for one that cannot be used at all.
    pub aim: Option<Aim>,
    /// Whether it is marked to be sold when it next reaches the shop.
    pub for_sale: bool,
    /// The seat that bought it.
    pub owner: SlotId,
}

/// An item lying on the ground that the viewing team can see.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LootView {
    /// Stable handle for this ground item.
    pub id: EntityId,
    /// Where it lies.
    pub pos: Vec2,
    /// Which item it is.
    pub item: ItemId,
    /// Charges left. Absent for an item that has no charges at all.
    pub charges: Option<u8>,
}

/// A unit the viewing team can currently see, or one of its own.
///
/// Every stat is the effective value, after buffs, items and auras.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct UnitView {
    /// Stable handle for this unit.
    pub id: EntityId,
    /// What kind of unit it is.
    pub kind: UnitKind,
    /// Which side it belongs to.
    pub team: Team,
    /// Current position.
    pub pos: Vec2,
    /// Which way it is facing.
    pub facing: Angle,
    /// Current health, whole points. At least 1 while any is left.
    pub hp: i32,
    /// Maximum health.
    pub max_hp: i32,
    /// Current mana, whole points. At least 1 while any is left; zero for
    /// units that have none.
    pub mana: i32,
    /// Maximum mana. Zero for units that do not have any.
    pub max_mana: i32,
    /// Movement speed in world units per second.
    pub move_speed: Fixed,
    /// Attack damage per hit, before the target's armor.
    pub attack_damage: i32,
    /// Attack range in world units, edge to edge of the [`bound`](Self::bound)s.
    pub attack_range: Fixed,
    /// Milliseconds between the start of one attack and the next, after
    /// attack speed.
    pub attack_time: u32,
    /// Milliseconds from the start of an attack to the hit, after attack
    /// speed.
    pub attack_point: u32,
    /// Attack speed, where 100 is the unit's own pace and 200 is twice it.
    pub attack_speed: i32,
    /// Armor.
    pub armor: Fixed,
    /// Magic resistance as a fraction, where 1.0 is total immunity.
    pub magic_resist: Fixed,
    /// Collision size: how near another body's centre may come, less that
    /// body's own, in world units. Zero for what has no body.
    pub collision: Fixed,
    /// Bound radius: where its edge is for attack range, cast range and
    /// areas, in world units.
    pub bound: Fixed,
    /// How far this unit lights the fog for its own team, in world units.
    /// Zero if it lights none.
    pub vision_radius: Fixed,
    /// How far it reveals what hides. Zero for whatever gives no true sight.
    pub true_sight_radius: Fixed,
    /// Conditions currently affecting it.
    pub statuses: StatusFlags,
    /// The three attributes, after items. All zero for anything that has none.
    pub attributes: Attributes,
    /// Which attribute pays this unit attack damage. Absent for a unit whose
    /// damage answers to none of them.
    pub primary: Option<Attribute>,
    /// Which hero this is, when `kind` is [`UnitKind::Hero`].
    pub hero: Option<HeroId>,
    /// Which seat controls it: set for a hero and a courier.
    pub owner: Option<SlotId>,
    /// Hero level. Zero for units that do not level.
    pub level: u8,
    /// Ability slots, in slot order. Empty for anything but a hero or a
    /// courier.
    pub abilities: Vec<AbilityView>,
    /// Item slots in slot order: a hero's six inventory and three backpack
    /// slots, a courier's six. Empty for anything else.
    pub items: Vec<Option<ItemView>>,
    /// Effects currently on the unit, timed and counted alike.
    pub effects: Vec<EffectView>,
}

/// A missile in flight, or what an ability shows where it stands, that the
/// viewing team can see or launched itself.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProjectileView {
    /// Stable handle for this projectile.
    pub id: EntityId,
    /// Current position.
    pub pos: Vec2,
    /// Direction of travel.
    pub facing: Angle,
    /// Which side launched it.
    pub team: Team,
    /// Which ability it belongs to. Absent for a plain attack.
    pub ability: Option<AbilityId>,
}

/// The abilities and items of a seat whose hero is dead.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Kit {
    /// The ability slots, in slot order.
    pub abilities: Vec<AbilityView>,
    /// The inventory and backpack slots, in slot order.
    pub items: Vec<Option<ItemView>>,
}

/// The scoreboard entry for one seat.
///
/// Present for every seat in the match, including enemies. Fields that are
/// hidden from the viewing team are absent individually.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlayerView {
    /// Which seat this describes.
    pub slot: SlotId,
    /// Which side the seat plays for.
    pub team: Team,
    /// Which hero was picked. Set even while the hero is dead.
    pub hero: HeroId,
    /// The hero's unit. Absent while dead.
    pub unit: Option<EntityId>,
    /// Hero level.
    pub level: u8,
    /// Experience gathered over the whole match.
    pub xp: i32,
    /// Unspent gold. Absent for the opposing team.
    pub gold: Option<i32>,
    /// The six stash slots at the home shop, in slot order. Absent for the
    /// opposing team.
    pub stash: Option<Vec<Option<ItemView>>>,
    /// What the dead hero carried. Absent while the hero stands, and for the
    /// opposing team.
    pub kit: Option<Kit>,
    /// Kills scored.
    pub kills: u16,
    /// Times died.
    pub deaths: u16,
    /// Kills assisted.
    pub assists: u16,
    /// Enemy creeps last hit.
    pub last_hits: u16,
    /// Friendly creeps denied.
    pub denies: u16,
    /// Ticks until respawn. Zero when alive.
    pub respawn_left: u32,
}

/// Everything one team is allowed to know, as of one tick.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WorldView {
    /// Which tick this describes. Divide by
    /// [`MatchInfo::tick_rate`](crate::MatchInfo::tick_rate) for seconds since
    /// the start.
    pub tick: u32,
    /// Whose eyes this is through. Absent for a view of everything.
    pub viewer: Option<Team>,
    /// Every unit currently visible and every unit of the viewing team,
    /// sorted by [`EntityId`].
    pub units: Vec<UnitView>,
    /// Every projectile currently visible and every one the viewing team
    /// launched, sorted by [`EntityId`].
    pub projectiles: Vec<ProjectileView>,
    /// The scoreboard, one entry per seat, sorted by [`SlotId`].
    pub players: Vec<PlayerView>,
    /// Which of the map's own trees are down right now, as indices into
    /// [`MatchInfo::trees`](crate::MatchInfo::trees).
    pub felled_trees: Vec<u32>,
    /// Where every tree put up during the match stands.
    pub planted_trees: Vec<Vec2>,
    /// Every item lying on the ground currently visible, sorted by
    /// [`EntityId`].
    pub loot: Vec<LootView>,
}
