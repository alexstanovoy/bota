//! What one side is allowed to know about the world right now.
//!
//! A [`WorldView`] is the server's world projected through one team's fog of
//! war. It is the only shape of game state that leaves the server, and the same
//! type is delivered to humans and to bots.

use crate::{
    AbilityId, Aim, Angle, Attribute, Attributes, EffectId, EntityId, Fixed, HeroId, ItemId,
    SlotId, Team, UnitKind, Vec2,
};
use serde::{Deserialize, Serialize};

/// Conditions currently affecting a unit.
///
/// A unit can be under several at once, so this is a bit set. Compare against
/// the associated constants.
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
    /// Cannot cast abilities, but can still move and attack.
    pub const SILENCED: u16 = 1 << 1;
    /// Cannot move, but can still act.
    pub const ROOTED: u16 = 1 << 2;
    /// Cannot attack, but can still move and cast.
    pub const DISARMED: u16 = 1 << 3;
    /// Movement speed is reduced.
    pub const SLOWED: u16 = 1 << 4;
    /// Losing health over time.
    pub const DOT: u16 = 1 << 5;
    /// Invisible to the other team.
    pub const INVISIBLE: u16 = 1 << 6;
    /// Immune to magical damage and most disables.
    pub const MAGIC_IMMUNE: u16 = 1 << 7;
    /// Dead and waiting to respawn.
    pub const DEAD: u16 = 1 << 8;
    /// Cannot be attacked or damaged at all.
    pub const INVULNERABLE: u16 = 1 << 9;
}

/// One ability slot of a visible hero.
///
/// Present for enemy heroes as well as friendly ones.
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
///
/// The two counts are independent: an effect may run out, may be counted, may
/// do both, and one that does neither carries only its id.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectView {
    /// Which effect it is.
    pub id: EffectId,
    /// Ticks until it wears off. Absent for one that does not run out.
    pub ticks_left: Option<u32>,
    /// How many are held. Absent for one that is not counted.
    pub stacks: Option<u32>,
}

/// One inventory slot of a visible hero.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemView {
    /// Which item sits in this slot.
    pub id: ItemId,
    /// Charges left. Absent for an item that has no charges at all, so an
    /// empty stack and one that never held any are told apart.
    pub charges: Option<u8>,
    /// Ticks remaining before the item can be used again. Zero means ready.
    pub cooldown_left: u32,
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

/// A unit the viewing team can currently see.
///
/// Every stat is the effective value, after buffs, items and auras. A unit the
/// team cannot see is absent from the view rather than blanked out.
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
    /// Which way it is facing. Turning takes time, so this does not follow from
    /// the movement direction.
    pub facing: Angle,
    /// Current health.
    pub hp: i32,
    /// Maximum health.
    pub max_hp: i32,
    /// Current mana. Zero for units that do not have any.
    pub mana: i32,
    /// Maximum mana. Zero for units that do not have any.
    pub max_mana: i32,
    /// Movement speed in world units per second.
    pub move_speed: Fixed,
    /// Attack damage per hit, before the target's armor.
    pub attack_damage: i32,
    /// Attack range.
    pub attack_range: Fixed,
    /// Ticks between the start of one attack and the next.
    pub attack_interval: u32,
    /// Attack speed, where 100 is the unit's own pace and 200 is twice it.
    pub attack_speed: i32,
    /// Armor.
    pub armor: Fixed,
    /// Magic resistance as a fraction, where 1.0 is total immunity.
    pub magic_resist: Fixed,
    /// Radius the unit occupies, used for collision and hit detection.
    pub radius: Fixed,
    /// How far this unit lights the fog for its own team. Zero if it lights
    /// none.
    ///
    /// The fog itself is not on the wire; each side derives what it needs from
    /// this field and the positions in the view.
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
    /// Which seat controls it, when it is a hero or a hero's summon.
    pub owner: Option<SlotId>,
    /// Hero level. Zero for units that do not level.
    pub level: u8,
    /// Ability slots. Empty for anything that is not a hero.
    pub abilities: Vec<AbilityView>,
    /// The six inventory and three backpack slots, in slot order. Empty for
    /// anything that is not a hero.
    pub items: Vec<Option<ItemView>>,
    /// Effects currently on the unit, timed and counted alike.
    pub effects: Vec<EffectView>,
}

/// A projectile in flight that the viewing team can see.
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
    /// Which ability launched it. Absent for a plain attack.
    pub ability: Option<AbilityId>,
}

/// What a hero's body left behind while it is gone.
///
/// Abilities and items outlive the body they were carried on, so a seat with
/// no hero standing still has both. While one stands they ride in its
/// [`UnitView`] instead, and this is absent.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Kit {
    /// The ability slots, in the order they are shown.
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
    /// Experience towards the next level.
    pub xp: i32,
    /// Unspent gold. Absent for the opposing team.
    pub gold: Option<i32>,
    /// The six stash slots at the home shop, in slot order. Absent for the
    /// opposing team.
    pub stash: Option<Vec<Option<ItemView>>>,
    /// What the body left behind while it is gone. Absent while a hero
    /// stands, and absent for the opposing team, which is left with whatever
    /// it saw last.
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
///
/// Produced on the server by projecting the world through a team's fog of war,
/// and sent whole on every tick.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WorldView {
    /// Which tick this describes. Divide by the tick rate from
    /// [`MatchInfo`](crate::MatchInfo) for a clock time.
    pub tick: u32,
    /// Whose eyes this is through. Absent for a spectator seeing everything.
    pub viewer: Option<Team>,
    /// Every unit currently visible, sorted by [`EntityId`].
    pub units: Vec<UnitView>,
    /// Every projectile currently visible, sorted by [`EntityId`].
    pub projectiles: Vec<ProjectileView>,
    /// The scoreboard, one entry per seat, sorted by [`SlotId`].
    pub players: Vec<PlayerView>,
    /// Which of the map's own trees are down right now, by their place in the
    /// list [`MatchInfo`](crate::MatchInfo) carried at the start.
    pub felled_trees: Vec<u32>,
    /// Where every tree put up during the match stands.
    pub planted_trees: Vec<Vec2>,
    /// Every item lying on the ground currently visible, sorted by
    /// [`EntityId`].
    pub loot: Vec<LootView>,
}
