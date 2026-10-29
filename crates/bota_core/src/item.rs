//! The catalog's shape: what an item costs, what it carries, and what using
//! it does.
//!
//! Every entry answers to an [`ItemId`], which is its place in
//! [`ITEMS`](crate::ITEMS). What a use of an item does to a world is the
//! server's, found by the entry's [`UseSpec`].

use bota_proto::{Aim, Attribute, Attributes, Fixed, ItemId};

use crate::{ITEMS, Odds, UnitDef, rules};

/// Which pool an item mends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pool {
    Health,
    Mana,
}

/// What one item adds to whoever carries it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Carried {
    /// Attributes added.
    pub attributes: Attributes,
    /// Movement speed added by boots. Of several carried, the best counts.
    pub move_speed: i32,
    /// Movement speed added by a Wind Lace. Of several carried, the best
    /// counts, on top of the boots.
    pub lace_speed: i32,
    /// Attack damage added.
    pub damage: i32,
    /// Attack speed added.
    pub attack_speed: i32,
    /// Armor added.
    pub armor: Fixed,
    /// Maximum health added.
    pub hp: i32,
    /// Maximum mana added.
    pub mana: i32,
    /// Health per tick added.
    pub hp_regen: Fixed,
    /// Mana per tick added.
    pub mana_regen: Fixed,
    /// Attack damage added against lane and neutral creeps.
    pub damage_to_creeps: i32,
    /// Percent added to the base attack speed and to what agility adds.
    /// Attack speed from items and from what is on the carrier is left
    /// alone.
    pub base_attack_speed_pct: i32,
    /// Chance an attack at the carrier misses. Several carried stack as
    /// Dota's evasion does, each taking its share of what the others let
    /// through ([`Odds::or`]).
    pub evasion: Odds,
    /// Chance an attack of the carrier pierces: goes through evasion and an
    /// uphill miss, and lands bonus magical damage. Every item carried rolls
    /// its own ([`carried_pierces`]); a carrier's total holds none.
    pub pierce: Odds,
    /// Magical damage a pierce lands alongside the attack.
    pub pierce_damage: i32,
    /// Whether several of this item carried pierce as one: Dota's Monkey
    /// King Bar.
    pub pierce_unique: bool,
    /// Attack range added to a melee carrier and to nobody else.
    pub melee_range: i32,
    /// Percent added to the carrier's maximum mana, attributes included.
    pub mana_pct: i32,
}

/// Nothing carried at all, so an entry names only what it adds.
pub(crate) const NOTHING: Carried = Carried {
    attributes: Attributes::ZERO,
    move_speed: 0,
    lace_speed: 0,
    damage: 0,
    attack_speed: 0,
    armor: Fixed::ZERO,
    hp: 0,
    mana: 0,
    hp_regen: Fixed::ZERO,
    mana_regen: Fixed::ZERO,
    damage_to_creeps: 0,
    base_attack_speed_pct: 0,
    evasion: Odds::NEVER,
    pierce: Odds::NEVER,
    pierce_damage: 0,
    pierce_unique: false,
    melee_range: 0,
    mana_pct: 0,
};

/// Whole points of one attribute and none of the others.
pub(crate) const fn points(which: Attribute, count: i32) -> Attributes {
    points_with(which, count, 0)
}

/// Whole points of every attribute, and `count` rather than `rest` of one.
pub(crate) const fn points_with(which: Attribute, count: i32, rest: i32) -> Attributes {
    let (count, rest) = (Fixed::from_int(count), Fixed::from_int(rest));
    match which {
        Attribute::Strength => Attributes {
            strength: count,
            agility: rest,
            intelligence: rest,
        },
        Attribute::Agility => Attributes {
            strength: rest,
            agility: count,
            intelligence: rest,
        },
        Attribute::Intelligence => Attributes {
            strength: rest,
            agility: rest,
            intelligence: count,
        },
    }
}

/// Health or mana per tick, from whole points per second.
pub(crate) const fn per_second(points: i32) -> Fixed {
    Fixed::from_ratio(points, rules::TICKS_PER_SECOND as i32)
}

/// Health or mana per tick, from hundredths of a point per second.
pub(crate) const fn hundredths_per_second(hundredths: i32) -> Fixed {
    Fixed::from_ratio(hundredths, 100 * rules::TICKS_PER_SECOND as i32)
}

/// How many charges one use costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spends {
    /// None at all.
    Nothing,
    /// One of them.
    One,
    /// Every one it holds.
    All,
}

/// What one drink of an item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mend {
    /// Which pool it mends.
    pub pool: Pool,
    /// How much it mends over the whole of it.
    pub total: i32,
    /// How long it runs, in ticks.
    pub ticks: u32,
    /// How far it reaches, in world units.
    pub range: i32,
    /// Whether it takes a tree down to work.
    pub eats_a_tree: bool,
    /// Whether a blow that [`breaks_regen`](crate::breaks_regen) puts it out.
    pub breaks: bool,
}

/// What a use of an item does, with the numbers that tune it. The server
/// turns each variant into the change it makes to the world.
#[derive(Clone, Copy, Debug)]
pub enum UseSpec {
    /// Nothing: the item cannot be used.
    None,
    /// Mends a pool over time.
    Mend(Mend),
    /// Plants a tree that lasts `ticks`.
    PlantTree {
        /// How long the tree stands.
        ticks: u32,
        /// How far the user may plant it.
        range: i32,
    },
    FellTree {
        /// How far the user may fell it.
        range: i32,
    },
    /// Stands a ward of a kind.
    Ward {
        /// What the ward is.
        unit: &'static UnitDef,
        /// How long it stands.
        ticks: u32,
        /// How far the user may stand it.
        range: i32,
    },
    /// Stands the kind of ward the item holds and has switched to: a Ward
    /// Dispenser plants an Observer Ward or a Sentry Ward by the entries
    /// of those two items.
    Dispense,
    /// Reads a teleport to a building. Channels for the entry's `duration`
    /// and is owed one wait per user when the entry's `shared_wait` is set.
    Teleport,
    /// Switches the attribute the item is set to, round Dota's wheel:
    /// strength, intelligence, agility.
    SwitchMode,
    /// Lets the user walk through units.
    Phase {
        /// Movement speed added to a melee user, percent.
        pct: i32,
        /// Movement speed added to a ranged user, percent.
        ranged_pct: i32,
        /// How long it lasts.
        ticks: u32,
    },
    /// Carries the user to a spot.
    Blink {
        /// The farthest it carries.
        range: i32,
        /// How far it carries along the same line when aimed past `range`.
        overshoot: i32,
    },
    /// Restores health and mana for every charge held, and spends them all.
    Restore {
        /// Health per charge.
        hp_per_charge: i32,
        mana_per_charge: i32,
    },
    /// Restores mana to the user or the one it is aimed at.
    Replenish {
        /// Mana restored.
        mana: i32,
    },
    /// Throws a Blood Grenade.
    Grenade,
}

/// How a shop keeps an item sold from a limited stock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stock {
    /// How many it holds when the match starts.
    pub initial: u8,
    /// How many it holds at most.
    pub max: u8,
    /// Ticks it takes to gain one more while it holds fewer than `max`.
    pub restock_ticks: u32,
}

/// One set of parts an item is built from, and the attribute the build is
/// set to when it comes of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe<'a> {
    pub parts: &'a [ItemId],
    /// The attribute the built item is set to. Absent for one set to none.
    pub mode: Option<Attribute>,
}

/// Most parts one recipe of the catalog asks for: the server assembles a build
/// from parts held in a fixed set of slots this wide, and the catalog's
/// build-time checks refuse a row whose recipe asks for more.
pub const MAX_COMPONENTS: usize = 4;

/// One entry of the catalog.
#[derive(Clone, Copy, Debug)]
pub struct ItemDef {
    /// What it is called.
    pub name: &'static str,
    /// Price in gold.
    pub cost: i32,
    /// Uses it is bought with. Zero for one that is never used up.
    ///
    /// A stack left holding none is gone, unless it may gain them again
    /// through [`ItemDef::cast_charges`].
    pub charges: u8,
    /// Maximum merged charges; zero disables merging and per-charge value and regeneration.
    pub stack_limit: u8,
    /// Charges it may hold, each one gained from an enemy cast within
    /// [`rules::MAGIC_CHARGE_RANGE`]. Zero for one that gains none.
    pub cast_charges: u8,
    /// Ticks before it may be used again.
    pub cooldown: u32,
    /// Whether that wait is owed by whoever used it rather than by the stack:
    /// buying another does not buy a fresh wait.
    pub shared_wait: bool,
    /// Mana using it costs.
    pub mana_cost: i32,
    /// Health using it costs, never the last point.
    pub health_cost: i32,
    /// How each side's shop keeps it. Absent for one sold without limit.
    pub stock: Option<Stock>,
    /// Ticks of waiting a blow that [`breaks_regen`](crate::breaks_regen)
    /// puts on it. Zero for one that answers to no blow.
    pub breaks_on_damage: u32,
    /// Which attribute it is set to when bought. Absent for one that is set
    /// to none.
    pub mode: Option<Attribute>,
    /// Points of whichever attribute it is set to that it adds.
    pub mode_bonus: i32,
    /// What it is built from. Empty for one that is bought whole.
    pub components: &'static [ItemId],
    /// Other sets of parts it is built from as well, each setting the build
    /// to an attribute of its own. Empty for one with a single recipe.
    pub alternatives: &'static [Recipe<'static>],
    /// What it adds to whoever carries it; health regeneration is per charge when mergeable.
    pub carried: Carried,
    /// What it adds to a carrier with a ranged attack instead. Absent for
    /// one that adds the same to every carrier.
    pub ranged: Option<Carried>,
    /// Whether the entry is retired: its id keeps its place, but nothing
    /// sells, builds or carries it any more.
    pub retired: bool,
    /// How a use of it is aimed. Absent for one that cannot be used.
    pub aim: Option<Aim>,
    /// How far one use reaches, in world units. Zero for one that reaches
    /// nowhere of its own.
    pub range: i32,
    /// Charges one use costs.
    pub spends: Spends,
    /// Milliseconds of cast point. Zero: the effect lands in the same tick.
    pub point: u32,
    /// Milliseconds of animation after. Zero: the body is not held.
    pub backswing: u32,
    /// Ticks it runs on after the use. Zero: over at once.
    pub duration: u32,
    /// Whether a use wants mana missing, and takes its user as the target.
    pub mana_deficit: bool,
    /// Whether a use goes off beside whatever the body is doing and breaks
    /// none of it off: Dota's `DOTA_ABILITY_BEHAVIOR_IMMEDIATE`. A channel it
    /// does not ignore it still breaks. Only an item with no cast point,
    /// backswing or duration may.
    pub immediate: bool,
    /// Whether ordering it while a channel runs leaves the channel be:
    /// Dota's `DOTA_ABILITY_BEHAVIOR_IGNORE_CHANNEL`. Only an item with no
    /// cast point, backswing or duration may.
    pub ignores_channel: bool,
    /// Whether no shop sells it: it only ever comes of what a seat holds.
    pub unbuyable: bool,
    /// What using it does.
    pub effect: UseSpec,
}

impl ItemDef {
    /// What it adds to a carrier, ranged or not.
    pub const fn carried_by(&self, ranged: bool) -> &Carried {
        match &self.ranged {
            Some(carried) if ranged => carried,
            _ => &self.carried,
        }
    }

    /// Every set of parts it is built from: its components, set to its
    /// [`mode`](ItemDef::mode), then its alternatives. Nothing for one that
    /// is bought whole.
    pub fn recipes(&self) -> impl Iterator<Item = Recipe<'static>> + '_ {
        let first = (!self.components.is_empty()).then_some(Recipe {
            parts: self.components,
            mode: self.mode,
        });
        first.into_iter().chain(self.alternatives.iter().copied())
    }

    /// What a drink of it mends, if it is drunk.
    pub const fn mend(&self) -> Option<Mend> {
        match self.effect {
            UseSpec::Mend(mend) => Some(mend),
            _ => None,
        }
    }

    /// Health and mana restored per charge, if it restores.
    pub const fn restore(&self) -> Option<(i32, i32)> {
        match self.effect {
            UseSpec::Restore {
                hp_per_charge,
                mana_per_charge,
            } => Some((hp_per_charge, mana_per_charge)),
            _ => None,
        }
    }
}

/// An item that costs nothing, carries nothing and does nothing, so an entry
/// names only what sets it apart.
pub(crate) const PLAIN: ItemDef = ItemDef {
    name: "",
    cost: 0,
    charges: 0,
    stack_limit: 0,
    cast_charges: 0,
    cooldown: 0,
    shared_wait: false,
    mana_cost: 0,
    health_cost: 0,
    stock: None,
    breaks_on_damage: 0,
    mode: None,
    mode_bonus: 0,
    components: &[],
    alternatives: &[],
    carried: NOTHING,
    ranged: None,
    retired: false,
    aim: None,
    range: 0,
    spends: Spends::Nothing,
    point: 0,
    backswing: 0,
    duration: 0,
    mana_deficit: false,
    immediate: false,
    ignores_channel: false,
    unbuyable: false,
    effect: UseSpec::None,
};

/// Gold per Mango charge, bought one at a time.
pub const MANGO_COST: i32 = 65;
/// Maximum Mango charges in one slot.
pub const MANGO_STACK_MAX: u8 = 3;
/// Mana restored by one Mango charge, capped at the user's maximum.
pub const MANGO_MANA: i32 = 100;
/// Health a Healing Salve mends: `health_regen` 30 a second for
/// `buff_duration` 13 seconds.
pub const SALVE_HEAL: i32 = 390;
/// Ticks a Healing Salve mends over.
pub const SALVE_TICKS: u32 = 13 * rules::TICKS_PER_SECOND;
/// How far a Blink Dagger carries, in world units.
pub const BLINK_RANGE: i32 = 1200;
/// Where a Blink Dagger aimed past its range lands instead, along the same
/// line: `blink_range_clamp` 960.
pub const BLINK_OVERSHOOT_RANGE: i32 = 960;
/// Health per tick per Mango charge carried: 0.4 a second, truncated to 873
/// raw a tick.
pub const MANGO_HP_REGEN: Fixed = Fixed::from_ratio(2, 5 * rules::TICKS_PER_SECOND as i32);

/// Ticks a Blood Grenade waits between throws.
pub const GRENADE_COOLDOWN: u32 = 10 * rules::TICKS_PER_SECOND;
/// How far a Blood Grenade is thrown, in world units.
pub const GRENADE_RANGE: i32 = 900;

const _: () = {
    assert!(GRENADE_RANGE > rules::GRENADE_RADIUS);
    assert!(MANGO_HP_REGEN.raw > 0);
};

/// What one item is, or nothing if no such item exists or it is retired.
#[inline]
pub fn item_def(id: ItemId) -> Option<&'static ItemDef> {
    ITEMS.get(usize::from(id.0)).filter(|def| !def.retired)
}

/// The whole shop as the wire states it, in item id order, retired entries
/// left out.
pub fn shop_entries() -> Vec<bota_proto::ShopEntry> {
    ITEMS
        .iter()
        .enumerate()
        .filter(|(_, def)| !def.retired && !def.unbuyable)
        .map(|(index, def)| bota_proto::ShopEntry {
            id: ItemId(index as u16),
            cost: def.cost,
            components: def.components.to_vec(),
            alternatives: def
                .alternatives
                .iter()
                .map(|recipe| recipe.parts.to_vec())
                .collect(),
        })
        .collect()
}

/// One stack of an inventory as far as the sum of what is carried cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarriedItem {
    pub id: ItemId,
    /// Charges it holds.
    pub charges: u8,
    /// The attribute it is set to, for an item with a mode.
    pub mode: Option<Attribute>,
}

/// One pierce a carrier rolls on each of its attacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pierce {
    /// Its chance.
    pub chance: Odds,
    /// The magical damage it lands.
    pub damage: i32,
    /// The item it is.
    pub item: ItemId,
}

/// The pierces carried items roll, each with where it was given: as Dota's
/// Javelin-based items, every item that pierces rolls its own and several
/// may land on one attack, but several of an item that pierces alone (the
/// Monkey King Bar) roll once. Pass only the stacks that count: the
/// inventory proper, none muted.
pub fn carried_pierces(
    items: impl IntoIterator<Item = (usize, ItemId)>,
    ranged: bool,
) -> impl Iterator<Item = (usize, Pierce)> {
    let mut seen: Vec<ItemId> = Vec::new();
    items.into_iter().filter_map(move |(at, id)| {
        let carried = item_def(id)?.carried_by(ranged);
        if carried.pierce == Odds::NEVER || (carried.pierce_unique && seen.contains(&id)) {
            return None;
        }
        seen.push(id);
        Some((
            at,
            Pierce {
                chance: carried.pierce,
                damage: carried.pierce_damage,
                item: id,
            },
        ))
    })
}

/// What the items carried add up to, for a carrier with a ranged attack
/// or not: an item with a [`ranged`](ItemDef::ranged) variant adds that to
/// a ranged one. Pass only the stacks that count: the inventory proper,
/// none muted.
///
/// Of the boots carried only the fastest counts, and of the laces only the
/// fastest, the two added up.
pub fn carried_bonus(items: impl Iterator<Item = CarriedItem>, ranged: bool) -> Carried {
    let mut total = Carried::default();
    for stack in items {
        let Some(def) = item_def(stack.id) else {
            continue;
        };
        let carried = def.carried_by(ranged);
        total.attributes += carried.attributes;
        total.move_speed = total.move_speed.max(carried.move_speed);
        total.lace_speed = total.lace_speed.max(carried.lace_speed);
        total.damage += carried.damage;
        total.attack_speed += carried.attack_speed;
        total.armor += carried.armor;
        total.hp += carried.hp;
        total.mana += carried.mana;
        // A client passes the charges a view shows; a stream it cannot trust
        // must not panic it, and the server never exceeds the limit.
        let regen_charges = if def.stack_limit > 0 {
            i32::from(stack.charges.min(def.stack_limit))
        } else {
            1
        };
        total.hp_regen += carried.hp_regen * Fixed::from_int(regen_charges);
        total.mana_regen += carried.mana_regen;
        total.damage_to_creeps += carried.damage_to_creeps;
        total.base_attack_speed_pct += carried.base_attack_speed_pct;
        total.evasion = total.evasion.or(carried.evasion);
        total.melee_range += carried.melee_range;
        total.mana_pct += carried.mana_pct;
        // What an item is set to is worth points of that attribute alone.
        if let Some(mode) = stack.mode {
            total.attributes += points(mode, def.mode_bonus);
        }
    }
    total
}
