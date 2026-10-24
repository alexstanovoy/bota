//! What one side is allowed to know about the world right now.

use crate::{
    AbilityId, Aim, Angle, Attribute, Attributes, EffectId, EntityId, EventKind, Fixed, HeroId,
    Inflictor, ItemId, NeutralId, Order, SlotId, TalentSlot, Team, UnitKind, Vec2,
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
    /// In the cast point of an ability or an item: a new order to the body
    /// breaks the cast off before anything is spent.
    pub const CASTING: u16 = 1 << 12;
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
    /// Whether a skill point could be spent on it right now. False on a
    /// unit of another side: its unspent points are its own.
    pub can_level: bool,
}

/// An effect currently on a visible unit.
///
/// As in Dota, it shows who put it on and with what while the viewer sees
/// that unit: the id it names is one of [`WorldView::units`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectView {
    pub id: EffectId,
    /// Ticks until it wears off. Absent for one that does not run out.
    pub ticks_left: Option<u32>,
    /// How many are held. Absent for one that is not counted.
    pub stacks: Option<u32>,
    /// The unit that put it on, its bearer for an innate. Absent for one no
    /// unit put on and when that unit is not in the view.
    pub caster: Option<EntityId>,
    /// The ability or item it came from. Absent for one that came from
    /// neither, as the Glyph's.
    pub inflictor: Option<Inflictor>,
    /// Ticks it was put on for, or last renewed for: what `ticks_left` counts
    /// down from. Absent exactly when `ticks_left` is. Last on the wire.
    pub ticks_total: Option<u32>,
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
    /// Whether it is marked to be sold when it next reaches the shop. False
    /// on a unit of another side.
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
    pub item: ItemId,
    /// Charges left. Absent for an item that has no charges at all.
    pub charges: Option<u8>,
    /// Which sides see it. Told only in a view of the whole map (a replay,
    /// a fogless spectator); absent in a side's view. Last on the wire.
    pub seen_by: Option<SeenBy>,
}

/// Which sides' views hold something: what Dota's spectator vision toggle
/// reads. Told only in a view of the whole map, never in a side's view: Dota
/// does not tell a side whether the other sees its units.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SeenBy {
    pub radiant: bool,
    pub dire: bool,
}

impl SeenBy {
    /// Whether `team`'s view holds it; the jungle's side views nothing.
    pub const fn by(self, team: Team) -> bool {
        match team {
            Team::Radiant => self.radiant,
            Team::Dire => self.dire,
            Team::Neutral => false,
        }
    }
}

/// A unit the viewing team can currently see, or one of its own.
///
/// Every stat is the effective value, after buffs, items and auras.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct UnitView {
    /// Stable handle for this unit.
    pub id: EntityId,
    pub kind: UnitKind,
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
    /// Attack damage per hit, before the target's armor: the middle of the
    /// range a swing draws from, rounded down.
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
    pub armor: Fixed,
    /// Magic resistance as a fraction, where 1.0 is total immunity.
    pub magic_resist: Fixed,
    /// Collision size: how near another body's centre may come, less that
    /// body's own, in world units. Zero for what has no body.
    pub collision: Fixed,
    /// Bound radius: where its edge is for attack range, cast range and
    /// areas, in world units.
    pub bound: Fixed,
    /// How far this unit lights the fog for its own team at the moment, by
    /// day or by night, in world units. Zero if it lights none.
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
    /// The least damage a swing draws, before the target's armor and attack
    /// class: the base range's floor with every bonus added.
    pub attack_damage_min: i32,
    /// The most damage a swing draws, on the same terms. `attack_damage` is
    /// the middle of the two, rounded down.
    pub attack_damage_max: i32,
    /// Health regained each second, all sources summed: base, strength, items,
    /// auras, consumables and the fountain. Q16.16 points per second.
    pub hp_regen: Fixed,
    /// Mana regained each second, on the same terms as `hp_regen`.
    pub mana_regen: Fixed,
    /// What a hero's town portal slot,
    /// [`ItemSlot::TOWN_PORTAL`](crate::ItemSlot::TOWN_PORTAL), holds.
    /// Absent for an empty slot and for anything but a hero.
    pub town_portal: Option<ItemView>,
    /// Which neutral creep this is, when `kind` is
    /// [`UnitKind::CreepNeutral`].
    pub neutral: Option<NeutralId>,
    /// The cast point it stands in, as Dota shows its caster's animation:
    /// present exactly while [`StatusFlags::CASTING`] is set.
    pub cast: Option<CastView>,
    /// The orders waiting in its queue, the next to be taken up first, as
    /// they were given: told to its own side alone, empty for the other
    /// side's units and for a unit with nothing queued. At most
    /// `bota_core::rules::MAX_QUEUED_ORDERS`.
    pub queue: Vec<Order>,
    /// Status resistance as a fraction: the share taken off the durations
    /// of stuns, fears and slows put on it.
    pub status_resist: Fixed,
    /// Evasion as a fraction: the share of attacks at it that miss.
    pub evasion: Fixed,
    /// Which sides' views hold it: its own side's always, the other's while
    /// it sees it. Told only in a view of the whole map; absent in a side's
    /// view. Last on the wire.
    pub seen_by: Option<SeenBy>,
}

/// A cast in its cast point.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CastView {
    /// The ability being cast, which has an animation of its own. Absent for
    /// the use of an item, whose animation tells no item.
    pub ability: Option<AbilityId>,
    /// Ticks until it goes off, unless it is broken off first. At least 1.
    pub goes_off_in: u32,
}

/// A swing under way that has not struck yet.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SwingView {
    /// Whom it strikes. Always one of [`WorldView::units`].
    pub target: EntityId,
    /// Ticks until it strikes at the current attack speed, unless it is
    /// broken off first: a melee blow lands, a ranged missile leaves. At
    /// least 1.
    pub strike_in: u32,
}

/// Whom a unit in the view fights, and how firmly it holds to that.
///
/// Every id it names is one of [`WorldView::units`], and it tells nothing of
/// a unit the view does not carry: a unit without an entry is set on nobody
/// the view carries, is not swinging at one and holds no timer on one.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AggroView {
    /// The unit this describes.
    pub unit: EntityId,
    /// Whom it is set on: what it attacks or walks up to attack. Absent when
    /// it is set on nobody, or on a unit this view does not carry.
    pub target: Option<EntityId>,
    /// Its swing in the windup. Absent between swings, in the recovery after
    /// one, and when the swing is at a unit this view does not carry.
    pub swing: Option<SwingView>,
    /// Ticks a lane creep keeps its target whatever else turns up, after an
    /// enemy hero's attack order drew it. Zero when nothing holds it, and
    /// when `target` is absent: a hold on a unit the view does not carry is
    /// not told.
    pub hold_left: u32,
    /// Ticks a lane creep keeps chasing its target out of attack range while
    /// nothing else is within its acquisition range. Full while the target
    /// is within reach; zero when it is set on nobody, and when `target` is
    /// absent.
    pub chase_left: u32,
    /// For a hero, told to its own side only: ticks before its attack order
    /// at an enemy hero may draw lane creeps again. For a tower: ticks before
    /// an enemy hero's attack order may draw it again. Zero when the next one
    /// does, and always zero for anything else.
    pub draw_cooldown: u32,
    /// Whether this neutral walks back to its camp and takes nothing on.
    pub returning: bool,
}

/// A missile in flight, or what an ability shows where it stands, that the
/// viewing team can see or launched itself.
///
/// Every id it names is one of [`WorldView::units`]: as in Dota, a missile
/// shows who let it go and whom it homes on while the viewer sees them.
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
    /// Retired: always absent. [`inflictor`](Self::inflictor) names what let
    /// it go.
    pub ability: Option<AbilityId>,
    /// Who let it go, or whose ability shows it. Absent when that unit is not
    /// in the view.
    pub source: Option<EntityId>,
    /// Whom it homes on. Absent for one that flies along a line or stands
    /// where it was put, and when that unit is not in the view.
    pub target: Option<EntityId>,
    /// The ability or item it belongs to. Absent for a plain attack.
    pub inflictor: Option<Inflictor>,
    /// Which sides' views hold it. Told only in a view of the whole map;
    /// absent in a side's view. Last on the wire.
    pub seen_by: Option<SeenBy>,
}

/// The abilities and items of a seat whose hero is dead.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Kit {
    /// The ability slots, in slot order.
    pub abilities: Vec<AbilityView>,
    /// The inventory and backpack slots, in slot order.
    pub items: Vec<Option<ItemView>>,
    /// What the town portal slot holds. Absent when it is empty.
    pub town_portal: Option<ItemView>,
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
    /// The hero's unit. Absent while dead, and for the other side while
    /// illusions of the hero stand, so that it cannot tell the hero from
    /// them (see the [`contract`](crate::contract)).
    pub unit: Option<EntityId>,
    /// Hero level. For a seat of the other side, the level its hero had
    /// when the viewing side last saw it (one before it ever did), as Dota's
    /// scoreboard shows it.
    pub level: u8,
    /// Experience gathered over the whole match. For a seat of the other
    /// side, on the terms of `level`.
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
    /// Enemy creeps last hit. Zero for the opposing team.
    pub last_hits: u16,
    /// Friendly creeps denied. Zero for the opposing team.
    pub denies: u16,
    /// Ticks until respawn. Zero when alive.
    pub respawn_left: u32,
}

/// How late one seat's orders take effect, as the server measured it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OrderDelayView {
    pub slot: SlotId,
    /// Ticks the latest applied order of this seat that named its view took
    /// effect after the earliest tick it could have: zero for an order given on
    /// the view of tick `T` that took effect on tick `T + 1`.
    pub ticks: u32,
}

/// One tick whose snapshot a connection was never sent, with the events of it
/// the connection may know.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SkippedTick {
    pub tick: u32,
    /// Its events, in the order the simulation produced them.
    pub events: Vec<EventKind>,
}

/// What one side's shop has left of an item sold from a limited stock.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StockView {
    /// Whose shop.
    pub team: Team,
    pub item: ItemId,
    /// How many may be bought right now. Zero refuses a purchase.
    pub left: u8,
    /// How many the shop holds at most.
    pub max: u8,
    /// Ticks until the shop gains one more. Zero while it holds `max`.
    pub restock_left: u32,
}

/// One side's Glyph of Fortification. A side is told its own in full; of the
/// other side's it is told only that it is active, `ready_in` being zero, so
/// the other side's cooldown stays hidden.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphView {
    /// The side whose Glyph it is.
    pub team: Team,
    /// Ticks until the side may activate it; zero when it is ready, and
    /// always zero in the other side's view.
    pub ready_in: u32,
    /// Ticks its structures still take no damage for; zero when it is not
    /// active.
    pub active_for: u32,
}

/// A teleport being channelled, as both sides see its destination.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TeleportView {
    /// The seat whose hero channels it; its hero is
    /// [`PlayerView::unit`] of that seat.
    pub slot: SlotId,
    /// The side it brings a hero to.
    pub team: Team,
    /// The allied building it was aimed at.
    pub building: EntityId,
    /// Where the hero lands when the channel runs out.
    pub destination: Vec2,
    /// Ticks until it lands, unless it is broken off first. At least 1.
    pub ticks_left: u32,
    /// Ticks the whole channel takes.
    pub ticks_total: u32,
}

/// A set of a hero's talent slots: bit `n` holds [`TalentSlot`]`(n)`.
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct Talents {
    /// The raw bits.
    pub bits: u8,
}

impl Talents {
    /// Whether the set holds a slot.
    pub const fn has(self, slot: TalentSlot) -> bool {
        slot.0 < TalentSlot::COUNT && self.bits & (1 << slot.0) != 0
    }

    /// The set with a slot added.
    pub const fn with(self, slot: TalentSlot) -> Talents {
        assert!(slot.0 < TalentSlot::COUNT);
        Talents {
            bits: self.bits | (1 << slot.0),
        }
    }
}

/// The talents of one seat's hero.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TalentView {
    pub slot: SlotId,
    pub learned: Talents,
    /// The talents that could be taken right now. Empty for a seat of the
    /// other side and for a hero that is down.
    pub learnable: Talents,
    /// Levels of the Attribute Bonus learned, up to seven.
    pub attribute_bonus: u8,
    /// Whether a skill point could go into the Attribute Bonus right now.
    /// False for a seat of the other side and for a hero that is down.
    pub attribute_bonus_learnable: bool,
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
    /// Which of the map's own trees are down, as indices into
    /// [`MatchInfo::trees`](crate::MatchInfo::trees), ascending: as the
    /// viewing side last saw them, or right now in a view of everything.
    /// A tree felled, or grown back, where the side has no sight is told
    /// once it sees that tree's ground again.
    pub felled_trees: Vec<u32>,
    /// Where the trees put up during the match stand, on the same terms:
    /// those the viewing side saw put up and has not seen gone, in the order
    /// it first saw them; every one standing in a view of everything.
    pub planted_trees: Vec<Vec2>,
    /// Every item lying on the ground currently visible, sorted by
    /// [`EntityId`].
    pub loot: Vec<LootView>,
    /// Whom the units fight, at most one entry a unit of
    /// [`units`](Self::units), sorted by [`EntityId`].
    pub aggro: Vec<AggroView>,
    /// The order delay of every seat this view is through whose applied
    /// orders named the view they were given on, sorted by [`SlotId`]. Empty
    /// until such an order takes effect.
    pub order_delays: Vec<OrderDelayView>,
    /// Every tick after the previous snapshot the receiver was sent and before
    /// this one, oldest first, each with its events: a receiver too slow for
    /// every snapshot is sent the latest one. Empty when none was skipped.
    pub skipped_ticks: Vec<SkippedTick>,
    /// What the shop has left of each item sold from a limited stock, for
    /// the viewing side, or for both sides in a view of everything; sorted
    /// by [`Team`], then by [`ItemId`].
    pub stock: Vec<StockView>,
    /// The talents of every seat of the viewing side whose hero has a tree,
    /// fallen or standing, and of every other seat whose hero is one of
    /// [`units`](Self::units); every seat with a tree in a view of
    /// everything. Sorted by [`SlotId`].
    pub talents: Vec<TalentView>,
    /// Every teleport being channelled, whichever side reads it: Dota shows
    /// a teleport's destination to both sides through the fog. Sorted by
    /// [`SlotId`].
    pub teleports: Vec<TeleportView>,
    /// The Glyph of Fortification of both sides, sorted by [`Team`]: the
    /// viewing side's in full, the other's without its cooldown. Last on the
    /// wire.
    pub glyphs: Vec<GlyphView>,
}

impl WorldView {
    /// Calls `visit` on every entity handle the view carries, those of its
    /// skipped ticks' events included: the one walk that puts a side's own
    /// names into a view, so a field that adds a handle is walked here.
    pub fn visit_ids(&mut self, visit: &mut dyn FnMut(&mut EntityId)) {
        for unit in &mut self.units {
            visit(&mut unit.id);
            for order in &mut unit.queue {
                order.visit_ids(visit);
            }
            for effect in &mut unit.effects {
                effect.caster.iter_mut().for_each(&mut *visit);
            }
        }
        for shot in &mut self.projectiles {
            visit(&mut shot.id);
            shot.source.iter_mut().for_each(&mut *visit);
            shot.target.iter_mut().for_each(&mut *visit);
        }
        for player in &mut self.players {
            player.unit.iter_mut().for_each(&mut *visit);
        }
        for loot in &mut self.loot {
            visit(&mut loot.id);
        }
        for entry in &mut self.aggro {
            visit(&mut entry.unit);
            entry.target.iter_mut().for_each(&mut *visit);
            if let Some(swing) = &mut entry.swing {
                visit(&mut swing.target);
            }
        }
        for teleport in &mut self.teleports {
            visit(&mut teleport.building);
        }
        for skipped in &mut self.skipped_ticks {
            for event in &mut skipped.events {
                event.visit_ids(visit);
            }
        }
    }

    /// The unit with this id, if it is in [`units`](Self::units).
    pub fn unit(&self, id: EntityId) -> Option<&UnitView> {
        let at = self.units.binary_search_by_key(&id, |u| u.id).ok()?;
        self.units.get(at)
    }

    /// The scoreboard entry of this seat, if it is in [`players`](Self::players).
    pub fn player(&self, slot: SlotId) -> Option<&PlayerView> {
        let at = self.players.binary_search_by_key(&slot, |p| p.slot).ok()?;
        self.players.get(at)
    }
}
