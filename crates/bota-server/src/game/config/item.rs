//! The catalog: what an item costs, what it carries, and what using it does.
//!
//! Every entry answers to an [`ItemId`], which is its place in [`ITEMS`].

use bota_proto::{Aim, Attribute, Attributes, Fixed, ItemId, ItemView};

use crate::game::Inventory;

use crate::game::UnitDef;
use crate::game::rules;

/// Which pool an item mends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pool {
    /// Health.
    Health,
    /// Mana.
    Mana,
}

/// What one item adds to whoever carries it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Carried {
    /// Attributes added.
    pub attributes: Attributes,
    /// Movement speed added.
    pub move_speed: i32,
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
    /// Attack damage added against anything that is not a hero.
    pub damage_to_creeps: i32,
}

/// Nothing carried at all, so an entry names only what it adds.
const NOTHING: Carried = Carried {
    attributes: Attributes::ZERO,
    move_speed: 0,
    damage: 0,
    attack_speed: 0,
    armor: Fixed::ZERO,
    hp: 0,
    mana: 0,
    hp_regen: Fixed::ZERO,
    mana_regen: Fixed::ZERO,
    damage_to_creeps: 0,
};

/// Whole points of one attribute and none of the others.
const fn points(which: Attribute, count: i32) -> Attributes {
    points_with(which, count, 0)
}

/// Whole points of every attribute, and `count` rather than `rest` of one.
const fn points_with(which: Attribute, count: i32, rest: i32) -> Attributes {
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
const fn per_second(points: i32) -> Fixed {
    Fixed::from_ratio(points, rules::TICKS_PER_SECOND as i32)
}

/// What using an item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemUse {
    /// Mends a unit over time.
    Mend {
        /// Which pool it mends.
        pool: Pool,
        /// How much it mends over the whole of it.
        total: i32,
        /// How long it runs.
        ticks: u32,
        /// How far it reaches, in world units.
        range: i32,
        /// Whether it takes a tree down to work, and needs one in reach.
        eats_a_tree: bool,
        /// Whether a blow from a hero or a tower puts it out.
        breaks: bool,
    },
    /// Mends whoever used it at once, spending every charge it holds.
    Restore {
        /// Health one charge mends.
        hp_per_charge: i32,
        /// Mana one charge mends.
        mana_per_charge: i32,
    },
    /// Stands a ward at a point.
    Ward {
        /// What kind of ward it stands.
        def: &'static UnitDef,
        /// How long the ward stands.
        ticks: u32,
        /// How far it reaches, in world units.
        range: i32,
    },
    /// Takes a tree down.
    Fell {
        /// How far it reaches, in world units.
        range: i32,
    },
    /// Puts a tree up, to stand for a while.
    Plant {
        /// Ticks it stands before it goes on its own.
        ticks: u32,
        /// How far it reaches, in world units.
        range: i32,
    },
    /// Carries whoever used it to an allied building.
    Teleport {
        /// Ticks of channelling before it carries.
        channel: u32,
        /// How far from an allied building it may land, in world units.
        range: i32,
    },
    /// Carries whoever used it to a point on open ground.
    Blink {
        /// How far it carries, in world units.
        range: i32,
    },
    /// Walks its user faster, and through the bodies in the way.
    Phase {
        /// Percent added to movement speed.
        pct: i32,
        /// How long it holds.
        ticks: u32,
    },
    /// Sets the item to the next attribute.
    Switch,
}

/// One entry of the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemDef {
    /// Price in gold.
    pub cost: i32,
    /// Uses it is bought with. Zero for one that is never used up.
    ///
    /// A stack left holding none is gone, unless it may gain them again
    /// through [`ItemDef::cast_charges`].
    pub charges: u8,
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
    /// Ticks of waiting a blow from a hero or a tower puts on it.
    /// Zero for one that answers to no blow.
    pub breaks_on_damage: u32,
    /// Which attribute it is set to when bought. Absent for one that is set
    /// to none.
    pub mode: Option<Attribute>,
    /// Points of whichever attribute it is set to that it adds.
    pub mode_bonus: i32,
    /// What it is built from. Empty for one that is bought whole.
    pub components: &'static [ItemId],
    /// What it adds to whoever carries it.
    pub carried: Carried,
    /// What using it does. Absent for one that cannot be used.
    pub active: Option<ItemUse>,
}

/// An item that costs nothing, carries nothing and does nothing, so an entry
/// names only what sets it apart.
const PLAIN: ItemDef = ItemDef {
    cost: 0,
    charges: 0,
    cast_charges: 0,
    cooldown: 0,
    shared_wait: false,
    mana_cost: 0,
    breaks_on_damage: 0,
    mode: None,
    mode_bonus: 0,
    components: &[],
    carried: NOTHING,
    active: None,
};

/// Boots of Speed.
pub const ITEM_BOOTS: u16 = 0;
/// Clarity.
pub const ITEM_CLARITY: u16 = 1;
/// Healing Salve.
pub const ITEM_HEALING_SALVE: u16 = 2;
/// Iron Branch.
pub const ITEM_IRON_BRANCH: u16 = 3;
/// Observer Ward.
pub const ITEM_OBSERVER_WARD: u16 = 4;
/// Quelling Blade.
pub const ITEM_QUELLING_BLADE: u16 = 5;
/// Sentry Ward.
pub const ITEM_SENTRY_WARD: u16 = 6;
/// Tango.
pub const ITEM_TANGO: u16 = 7;
/// Town Portal Scroll.
pub const ITEM_TOWN_PORTAL_SCROLL: u16 = 8;
/// Circlet.
pub const ITEM_CIRCLET: u16 = 9;
/// Gauntlets of Strength.
pub const ITEM_GAUNTLETS: u16 = 10;
/// Slippers of Agility.
pub const ITEM_SLIPPERS: u16 = 11;
/// Mantle of Intelligence.
pub const ITEM_MANTLE: u16 = 12;
/// Belt of Strength.
pub const ITEM_BELT: u16 = 13;
/// Band of Elvenskin.
pub const ITEM_BAND: u16 = 14;
/// Robe of the Magi.
pub const ITEM_ROBE: u16 = 15;
/// Ogre Axe.
pub const ITEM_OGRE_AXE: u16 = 16;
/// Blade of Alacrity.
pub const ITEM_BLADE_OF_ALACRITY: u16 = 17;
/// Staff of Wizardry.
pub const ITEM_STAFF_OF_WIZARDRY: u16 = 18;
/// Gloves of Haste.
pub const ITEM_GLOVES: u16 = 19;
/// Blades of Attack.
pub const ITEM_BLADES_OF_ATTACK: u16 = 20;
/// Broadsword.
pub const ITEM_BROADSWORD: u16 = 21;
/// Quarterstaff.
pub const ITEM_QUARTERSTAFF: u16 = 22;
/// Ring of Protection.
pub const ITEM_RING_OF_PROTECTION: u16 = 23;
/// Chainmail.
pub const ITEM_CHAINMAIL: u16 = 24;
/// Ring of Regen.
pub const ITEM_RING_OF_REGEN: u16 = 25;
/// Sage's Mask.
pub const ITEM_SAGES_MASK: u16 = 26;
/// Vitality Booster.
pub const ITEM_VITALITY_BOOSTER: u16 = 27;
/// Energy Booster.
pub const ITEM_ENERGY_BOOSTER: u16 = 28;
/// Power Treads.
pub const ITEM_POWER_TREADS: u16 = 29;
/// Phase Boots.
pub const ITEM_PHASE_BOOTS: u16 = 30;
/// Blink Dagger.
pub const ITEM_BLINK_DAGGER: u16 = 31;
/// Bracer.
pub const ITEM_BRACER: u16 = 32;
/// Wraith Band.
pub const ITEM_WRAITH_BAND: u16 = 33;
/// Null Talisman.
pub const ITEM_NULL_TALISMAN: u16 = 34;
/// Magic Stick.
pub const ITEM_MAGIC_STICK: u16 = 35;
/// Magic Wand.
pub const ITEM_MAGIC_WAND: u16 = 36;
/// The recipe Phase Boots are built with.
pub const ITEM_RECIPE_PHASE_BOOTS: u16 = 37;
/// The recipe a Bracer is built with.
pub const ITEM_RECIPE_BRACER: u16 = 38;
/// The recipe a Wraith Band is built with.
pub const ITEM_RECIPE_WRAITH_BAND: u16 = 39;
/// The recipe a Null Talisman is built with.
pub const ITEM_RECIPE_NULL_TALISMAN: u16 = 40;
/// The recipe a Magic Wand is built with.
pub const ITEM_RECIPE_MAGIC_WAND: u16 = 41;

/// What Power Treads are built from.
const TREADS_PARTS: [ItemId; 3] = [ItemId(ITEM_BOOTS), ItemId(ITEM_GLOVES), ItemId(ITEM_BELT)];
/// What Phase Boots are built from.
const PHASE_PARTS: [ItemId; 4] = [
    ItemId(ITEM_BOOTS),
    ItemId(ITEM_BLADES_OF_ATTACK),
    ItemId(ITEM_BLADES_OF_ATTACK),
    ItemId(ITEM_RECIPE_PHASE_BOOTS),
];
/// What a Bracer is built from.
const BRACER_PARTS: [ItemId; 3] = [
    ItemId(ITEM_CIRCLET),
    ItemId(ITEM_GAUNTLETS),
    ItemId(ITEM_RECIPE_BRACER),
];
/// What a Wraith Band is built from.
const WRAITH_PARTS: [ItemId; 3] = [
    ItemId(ITEM_CIRCLET),
    ItemId(ITEM_SLIPPERS),
    ItemId(ITEM_RECIPE_WRAITH_BAND),
];
/// What a Null Talisman is built from.
const NULL_PARTS: [ItemId; 3] = [
    ItemId(ITEM_CIRCLET),
    ItemId(ITEM_MANTLE),
    ItemId(ITEM_RECIPE_NULL_TALISMAN),
];
/// What a Magic Wand is built from.
const WAND_PARTS: [ItemId; 4] = [
    ItemId(ITEM_MAGIC_STICK),
    ItemId(ITEM_IRON_BRANCH),
    ItemId(ITEM_IRON_BRANCH),
    ItemId(ITEM_RECIPE_MAGIC_WAND),
];

/// The catalog, indexed by [`ItemId`].
pub const ITEMS: [ItemDef; 42] = [
    // Boots of Speed.
    ItemDef {
        cost: 500,
        carried: Carried {
            move_speed: 45,
            ..NOTHING
        },
        ..PLAIN
    },
    // Clarity.
    ItemDef {
        cost: 50,
        charges: 1,
        active: Some(ItemUse::Mend {
            pool: Pool::Mana,
            total: 150,
            ticks: 750,
            range: 250,
            eats_a_tree: false,
            breaks: true,
        }),
        ..PLAIN
    },
    // Healing Salve.
    ItemDef {
        cost: 110,
        charges: 1,
        active: Some(ItemUse::Mend {
            pool: Pool::Health,
            total: 400,
            ticks: 300,
            range: 250,
            eats_a_tree: false,
            breaks: true,
        }),
        ..PLAIN
    },
    // Iron Branch.
    ItemDef {
        cost: 50,
        charges: 1,
        carried: Carried {
            damage: 1,
            hp: 30,
            mana: 15,
            ..NOTHING
        },
        active: Some(ItemUse::Plant {
            ticks: rules::PLANTED_TREE_TICKS,
            range: 350,
        }),
        ..PLAIN
    },
    // Observer Ward.
    ItemDef {
        cost: 100,
        charges: 1,
        active: Some(ItemUse::Ward {
            def: &crate::game::OBSERVER_WARD,
            ticks: 10800,
            range: 500,
        }),
        ..PLAIN
    },
    // Quelling Blade.
    ItemDef {
        cost: 225,
        cooldown: 120,
        carried: Carried {
            damage_to_creeps: 18,
            ..NOTHING
        },
        active: Some(ItemUse::Fell { range: 350 }),
        ..PLAIN
    },
    // Sentry Ward.
    ItemDef {
        cost: 50,
        charges: 1,
        active: Some(ItemUse::Ward {
            def: &crate::game::SENTRY_WARD,
            ticks: 12600,
            range: 500,
        }),
        ..PLAIN
    },
    // Tango.
    ItemDef {
        cost: 90,
        charges: 3,
        active: Some(ItemUse::Mend {
            pool: Pool::Health,
            total: 115,
            ticks: 480,
            range: 165,
            eats_a_tree: true,
            breaks: false,
        }),
        ..PLAIN
    },
    // Town Portal Scroll.
    ItemDef {
        cost: 100,
        charges: 1,
        cooldown: rules::SCROLL_WAIT_TICKS,
        shared_wait: true,
        active: Some(ItemUse::Teleport {
            channel: 90,
            range: 600,
        }),
        ..PLAIN
    },
    // Circlet.
    ItemDef {
        cost: 155,
        carried: Carried {
            attributes: Attributes::all(2),
            ..NOTHING
        },
        ..PLAIN
    },
    // Gauntlets of Strength.
    ItemDef {
        cost: 140,
        carried: Carried {
            attributes: points(Attribute::Strength, 3),
            ..NOTHING
        },
        ..PLAIN
    },
    // Slippers of Agility.
    ItemDef {
        cost: 140,
        carried: Carried {
            attributes: points(Attribute::Agility, 3),
            ..NOTHING
        },
        ..PLAIN
    },
    // Mantle of Intelligence.
    ItemDef {
        cost: 140,
        carried: Carried {
            attributes: points(Attribute::Intelligence, 3),
            ..NOTHING
        },
        ..PLAIN
    },
    // Belt of Strength.
    ItemDef {
        cost: 450,
        carried: Carried {
            attributes: points(Attribute::Strength, 6),
            ..NOTHING
        },
        ..PLAIN
    },
    // Band of Elvenskin.
    ItemDef {
        cost: 450,
        carried: Carried {
            attributes: points(Attribute::Agility, 6),
            ..NOTHING
        },
        ..PLAIN
    },
    // Robe of the Magi.
    ItemDef {
        cost: 450,
        carried: Carried {
            attributes: points(Attribute::Intelligence, 6),
            ..NOTHING
        },
        ..PLAIN
    },
    // Ogre Axe.
    ItemDef {
        cost: 1000,
        carried: Carried {
            attributes: points(Attribute::Strength, 10),
            ..NOTHING
        },
        ..PLAIN
    },
    // Blade of Alacrity.
    ItemDef {
        cost: 1000,
        carried: Carried {
            attributes: points(Attribute::Agility, 10),
            ..NOTHING
        },
        ..PLAIN
    },
    // Staff of Wizardry.
    ItemDef {
        cost: 1000,
        carried: Carried {
            attributes: points(Attribute::Intelligence, 10),
            ..NOTHING
        },
        ..PLAIN
    },
    // Gloves of Haste.
    ItemDef {
        cost: 450,
        carried: Carried {
            attack_speed: 20,
            ..NOTHING
        },
        ..PLAIN
    },
    // Blades of Attack.
    ItemDef {
        cost: 450,
        carried: Carried {
            damage: 9,
            ..NOTHING
        },
        ..PLAIN
    },
    // Broadsword.
    ItemDef {
        cost: 1000,
        carried: Carried {
            damage: 18,
            ..NOTHING
        },
        ..PLAIN
    },
    // Quarterstaff.
    ItemDef {
        cost: 875,
        carried: Carried {
            damage: 10,
            attack_speed: 10,
            ..NOTHING
        },
        ..PLAIN
    },
    // Ring of Protection.
    ItemDef {
        cost: 175,
        carried: Carried {
            armor: Fixed::from_int(2),
            ..NOTHING
        },
        ..PLAIN
    },
    // Chainmail.
    ItemDef {
        cost: 550,
        carried: Carried {
            armor: Fixed::from_int(5),
            ..NOTHING
        },
        ..PLAIN
    },
    // Ring of Regen.
    ItemDef {
        cost: 175,
        carried: Carried {
            hp_regen: per_second(2),
            ..NOTHING
        },
        ..PLAIN
    },
    // Sage's Mask.
    ItemDef {
        cost: 175,
        carried: Carried {
            mana_regen: per_second(1),
            ..NOTHING
        },
        ..PLAIN
    },
    // Vitality Booster.
    ItemDef {
        cost: 1000,
        carried: Carried { hp: 250, ..NOTHING },
        ..PLAIN
    },
    // Energy Booster.
    ItemDef {
        cost: 900,
        carried: Carried {
            mana: 250,
            ..NOTHING
        },
        ..PLAIN
    },
    // Power Treads.
    ItemDef {
        cost: 1400,
        mode: Some(Attribute::Strength),
        mode_bonus: 10,
        components: &TREADS_PARTS,
        carried: Carried {
            move_speed: 45,
            attack_speed: 25,
            ..NOTHING
        },
        active: Some(ItemUse::Switch),
        ..PLAIN
    },
    // Phase Boots.
    ItemDef {
        cost: 1500,
        cooldown: 240,
        components: &PHASE_PARTS,
        carried: Carried {
            move_speed: 45,
            damage: 18,
            ..NOTHING
        },
        active: Some(ItemUse::Phase { pct: 20, ticks: 93 }),
        ..PLAIN
    },
    // Blink Dagger.
    ItemDef {
        cost: 2250,
        cooldown: 450,
        breaks_on_damage: 90,
        active: Some(ItemUse::Blink { range: 1200 }),
        ..PLAIN
    },
    // Bracer.
    ItemDef {
        cost: 505,
        components: &BRACER_PARTS,
        carried: Carried {
            attributes: points_with(Attribute::Strength, 6, 3),
            ..NOTHING
        },
        ..PLAIN
    },
    // Wraith Band.
    ItemDef {
        cost: 505,
        components: &WRAITH_PARTS,
        carried: Carried {
            attributes: points_with(Attribute::Agility, 6, 3),
            ..NOTHING
        },
        ..PLAIN
    },
    // Null Talisman.
    ItemDef {
        cost: 505,
        components: &NULL_PARTS,
        carried: Carried {
            attributes: points_with(Attribute::Intelligence, 6, 3),
            ..NOTHING
        },
        ..PLAIN
    },
    // Magic Stick.
    ItemDef {
        cost: 200,
        cast_charges: 10,
        cooldown: 390,
        active: Some(ItemUse::Restore {
            hp_per_charge: 15,
            mana_per_charge: 15,
        }),
        ..PLAIN
    },
    // Magic Wand.
    ItemDef {
        cost: 450,
        cast_charges: 20,
        cooldown: 390,
        components: &WAND_PARTS,
        carried: Carried {
            attributes: Attributes::all(3),
            ..NOTHING
        },
        active: Some(ItemUse::Restore {
            hp_per_charge: 15,
            mana_per_charge: 15,
        }),
        ..PLAIN
    },
    // The recipe Phase Boots are built with.
    ItemDef { cost: 100, ..PLAIN },
    // The recipe a Bracer is built with.
    ItemDef { cost: 210, ..PLAIN },
    // The recipe a Wraith Band is built with.
    ItemDef { cost: 210, ..PLAIN },
    // The recipe a Null Talisman is built with.
    ItemDef { cost: 210, ..PLAIN },
    // The recipe a Magic Wand is built with.
    ItemDef { cost: 150, ..PLAIN },
];

/// How one use of an item is aimed.
pub fn item_aim(use_of: ItemUse) -> Aim {
    match use_of {
        // What eats a tree is aimed at one; what is drunk is aimed at whoever
        // drinks it.
        ItemUse::Mend { eats_a_tree, .. } => {
            if eats_a_tree {
                Aim::Tree
            } else {
                Aim::Unit
            }
        }
        ItemUse::Fell { .. } => Aim::Tree,
        ItemUse::Ward { .. } | ItemUse::Plant { .. } | ItemUse::Blink { .. } => Aim::Point,
        ItemUse::Teleport { .. } => Aim::Building,
        ItemUse::Restore { .. } | ItemUse::Phase { .. } | ItemUse::Switch => Aim::Own,
    }
}

/// How far one use of an item reaches, in world units.
///
/// For one aimed at a building this is how far from that building it may
/// land, not how far its user may stand from it.
pub fn item_range(use_of: ItemUse) -> i32 {
    match use_of {
        ItemUse::Mend { range, .. }
        | ItemUse::Ward { range, .. }
        | ItemUse::Fell { range }
        | ItemUse::Plant { range, .. }
        | ItemUse::Teleport { range, .. }
        | ItemUse::Blink { range } => range,
        ItemUse::Restore { .. } | ItemUse::Phase { .. } | ItemUse::Switch => 0,
    }
}

/// What one item is, or nothing if no such item exists.
pub fn item_def(id: ItemId) -> Option<&'static ItemDef> {
    ITEMS.get(usize::from(id.0))
}

/// Every item built from a given one, in catalog order.
pub fn built_from(part: ItemId) -> impl Iterator<Item = ItemId> {
    ITEMS
        .iter()
        .enumerate()
        .filter(move |(_, def)| def.components.contains(&part))
        .map(|(index, _)| ItemId(index as u16))
}

/// The whole shop as the wire states it, in item id order.
pub fn shop_entries() -> Vec<bota_proto::ShopEntry> {
    ITEMS
        .iter()
        .enumerate()
        .map(|(index, def)| bota_proto::ShopEntry {
            id: ItemId(index as u16),
            cost: def.cost,
            components: def.components.to_vec(),
        })
        .collect()
}

/// What a bag looks like on the wire, an empty slot keeping its place.
pub fn item_views(bag: &Inventory) -> Vec<Option<ItemView>> {
    bag.slots
        .iter()
        .map(|slot| {
            slot.map(|stack| {
                let def = item_def(stack.id);
                ItemView {
                    id: stack.id,
                    charges: def
                        .filter(|def| def.charges > 0 || def.cast_charges > 0)
                        .map(|_| stack.charges),
                    cooldown_left: stack.cooldown,
                    mute_left: stack.mute,
                    mode: stack.mode,
                    mana_cost: def.map_or(0, |def| def.mana_cost),
                    range: def.and_then(|def| def.active).map_or(0, item_range),
                    aim: def.and_then(|def| def.active).map(item_aim),
                    for_sale: stack.for_sale,
                }
            })
        })
        .collect()
}
