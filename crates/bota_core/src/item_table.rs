//! The catalog itself: one entry to an item, in the order of the ids.
//!
//! Numbers are Dota 7.41f's: the game's `scripts/npc/items.txt` (client
//! 6957, 2026-10-09) for costs, cooldowns, ranges, recipes and what an item
//! adds, and Liquipedia where that file says nothing. A shop's stock is the
//! file's `ItemStockInitial`, `ItemStockMax` and `ItemStockTime`, kept for every
//! item that has one; the wards cost nothing and wait one second between
//! plantings.

use bota_proto::{Aim, Attribute, Attributes, Fixed, ItemId};

use crate::{
    BLINK_OVERSHOOT_RANGE, BLINK_RANGE, Carried, GRENADE_COOLDOWN, GRENADE_RANGE, ItemDef,
    MANGO_COST, MANGO_HP_REGEN, MANGO_MANA, MANGO_STACK_MAX, MAX_COMPONENTS, Mend, NOTHING,
    OBSERVER_WARD, Odds, PLAIN, Pool, Recipe, SALVE_HEAL, SALVE_TICKS, SENTRY_WARD, Spends, Stock,
    UseSpec, hundredths_per_second, per_second, points, points_with, rules,
};

// Stacking is the file's `ItemStackable` (with `ItemStackableMax` where it
// has one: the Mango 3, the Blood Grenade 2; none: a byte of charges here):
// Clarity, Healing Salve, Tango, Town Portal Scroll, the wards, Mango and
// Blood Grenade stack; Iron Branch and every other entry have none, and the
// Magic Stick and Wand keep their charges in `cast_charges`. Faerie Fire,
// Smoke of Deceit, Dust of Appearance and the Infused Raindrop are no part of
// the 1v1 catalog (the first and the Raindrop are `ItemStackable` 0).

/// What Power Treads are built from first, set to strength.
const TREADS_PARTS: [ItemId; 3] = [ItemId::BOOTS, ItemId::GLOVES, ItemId::BELT];
/// Power Treads' other recipes, `item_recipe_power_treads` requirements 04
/// and 06: a Robe sets them to intelligence and a Band to agility.
const TREADS_ALTERNATIVES: [Recipe<'static>; 2] = [
    Recipe {
        parts: &[ItemId::BOOTS, ItemId::GLOVES, ItemId::ROBE],
        mode: Some(Attribute::Intelligence),
    },
    Recipe {
        parts: &[ItemId::BOOTS, ItemId::GLOVES, ItemId::BAND],
        mode: Some(Attribute::Agility),
    },
];
/// What Phase Boots are built from: `item_recipe_phase_boots` costs
/// nothing and asks for Boots, Chainmail and Blades of Attack alone.
const PHASE_PARTS: [ItemId; 3] = [ItemId::BOOTS, ItemId::CHAINMAIL, ItemId::BLADES_OF_ATTACK];
/// What a Bracer is built from.
const BRACER_PARTS: [ItemId; 3] = [ItemId::CIRCLET, ItemId::GAUNTLETS, ItemId::RECIPE_BRACER];
/// What a Wraith Band is built from.
const WRAITH_PARTS: [ItemId; 3] = [
    ItemId::CIRCLET,
    ItemId::SLIPPERS,
    ItemId::RECIPE_WRAITH_BAND,
];
/// What a Null Talisman is built from.
const NULL_PARTS: [ItemId; 3] = [
    ItemId::CIRCLET,
    ItemId::MANTLE,
    ItemId::RECIPE_NULL_TALISMAN,
];
/// What a Magic Wand is built from.
const WAND_PARTS: [ItemId; 4] = [
    ItemId::MAGIC_STICK,
    ItemId::IRON_BRANCH,
    ItemId::IRON_BRANCH,
    ItemId::RECIPE_MAGIC_WAND,
];
/// What a Butterfly is built from.
const BUTTERFLY_PARTS: [ItemId; 3] = [
    ItemId::EAGLESONG,
    ItemId::CLAYMORE,
    ItemId::TALISMAN_OF_EVASION,
];
/// What a Monkey King Bar is built from.
const MKB_PARTS: [ItemId; 4] = [
    ItemId::DEMON_EDGE,
    ItemId::BLITZ_KNUCKLES,
    ItemId::JAVELIN,
    ItemId::RECIPE_MONKEY_KING_BAR,
];

/// The catalog, indexed by [`ItemId`].
///
/// # Adding an item
///
/// Every item:
///
/// 1. Register the id: `ItemId::X` in `bota_proto/src/ids.rs` and one line in
///    `bota_proto/registry/ids.txt` (append-only; tests pin both).
/// 2. Add its [`ItemDef`] here at the id's index (and grow the array): `name`
///    equals the registry line, the numbers come from the game's `items.txt`.
///    A recipe is two rows, the recipe scroll (a cost) and the build (its
///    `components`).
/// 3. Bump `RULES_VERSION` in `fingerprint.rs` and renew the digests in
///    `tests/fingerprint.rs`.
/// 4. Tests: `item_numbers.rs` in `bota_server` pins numbers; the determinism
///    pins re-record (the drivers buy by the length of this table) and, for a
///    stocked item, so do the dummy bench's digests.
/// 5. `bota_client_core/src/ui/shop.rs`: the shop tab (`group_of`) it sits in.
///    A front end keys its art and texts by the id (dbota: `src/faces.rs` and
///    `assets-manifest/ids.txt`).
///
/// A passive stat item that uses only the stats [`Carried`] has stops there.
/// Beyond it:
///
/// - A new stat: a [`Carried`] field with its `NOTHING` entry and its arm in
///   `carried_bonus` (`item.rs`), the server's `systems/stats.rs`, often a
///   `Stats` field.
/// - An active: a [`UseSpec`] variant holding its numbers, and its arm in
///   `use_effect` (`bota_server/src/game/config/item.rs`) with the effect as
///   an `impl World` function; a channel also needs its `use_goes_on`,
///   `use_breaks_off` and `use_completes` arms.
/// - A buff or debuff it applies: an `EffectId` and registry line, a
///   [`ModifierKind`](crate::ModifierKind) variant with its arms, an
///   [`EFFECTS`](crate::EFFECTS) row, and the server's `hash.rs`, `stats.rs`
///   and `systems/modifiers.rs` arms.
/// - A charge-spending consumable: the `charges`, `stack_limit`, `spends` and
///   `stock` fields; healing at once or reacting to damage needs a variant and
///   an arm.
///
/// Neutral items are not modelled yet. Every item gets its end-to-end test in
/// `bota_server/src/game/tests/`.
pub const ITEMS: [ItemDef; 55] = [
    ItemDef {
        name: "Boots of Speed",
        cost: 500,
        carried: Carried {
            move_speed: 45,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Clarity",
        cost: 60,
        charges: 1,
        stack_limit: u8::MAX,
        aim: Some(Aim::Unit),
        range: 250,
        spends: Spends::One,
        effect: UseSpec::Mend(Mend {
            pool: Pool::Mana,
            total: 150,
            ticks: 750,
            range: 250,
            eats_a_tree: false,
            breaks: true,
        }),
        stock: Some(Stock {
            initial: 5,
            max: 5,
            restock_ticks: 120 * rules::TICKS_PER_SECOND,
        }),
        ..PLAIN
    },
    ItemDef {
        name: "Healing Salve",
        cost: 100,
        charges: 1,
        stack_limit: u8::MAX,
        aim: Some(Aim::Unit),
        range: 250,
        spends: Spends::One,
        effect: UseSpec::Mend(Mend {
            pool: Pool::Health,
            total: SALVE_HEAL,
            ticks: SALVE_TICKS,
            range: 250,
            eats_a_tree: false,
            breaks: true,
        }),
        stock: Some(Stock {
            initial: 5,
            max: 5,
            restock_ticks: 120 * rules::TICKS_PER_SECOND,
        }),
        ..PLAIN
    },
    ItemDef {
        name: "Iron Branch",
        cost: 55,
        charges: 1,
        carried: Carried {
            attributes: Attributes::all(1),
            ..NOTHING
        },
        aim: Some(Aim::Point),
        range: 400,
        spends: Spends::One,
        effect: UseSpec::PlantTree {
            ticks: rules::PLANTED_TREE_TICKS,
            range: 400,
        },
        ..PLAIN
    },
    ItemDef {
        name: "Observer Ward",
        cost: 0,
        charges: 1,
        stack_limit: u8::MAX,
        aim: Some(Aim::Point),
        range: 500,
        spends: Spends::One,
        effect: UseSpec::Ward {
            unit: &OBSERVER_WARD,
            ticks: 10800,
            range: 500,
        },
        cooldown: 30,
        stock: Some(Stock {
            initial: 2,
            max: 4,
            restock_ticks: 135 * rules::TICKS_PER_SECOND,
        }),
        ..PLAIN
    },
    ItemDef {
        name: "Quelling Blade",
        cost: 100,
        cooldown: 120,
        carried: Carried {
            damage_to_creeps: 8,
            ..NOTHING
        },
        ranged: Some(Carried {
            damage_to_creeps: 4,
            ..NOTHING
        }),
        aim: Some(Aim::Tree),
        range: 350,
        effect: UseSpec::FellTree { range: 350 },
        ..PLAIN
    },
    ItemDef {
        name: "Sentry Ward",
        cost: 50,
        charges: 1,
        stack_limit: u8::MAX,
        aim: Some(Aim::Point),
        range: 500,
        spends: Spends::One,
        effect: UseSpec::Ward {
            unit: &SENTRY_WARD,
            ticks: 12600,
            range: 500,
        },
        cooldown: 30,
        stock: Some(Stock {
            initial: 3,
            max: 8,
            restock_ticks: 70 * rules::TICKS_PER_SECOND,
        }),
        ..PLAIN
    },
    ItemDef {
        name: "Tango",
        cost: 90,
        charges: 3,
        stack_limit: u8::MAX,
        aim: Some(Aim::Tree),
        range: 165,
        spends: Spends::One,
        effect: UseSpec::Mend(Mend {
            pool: Pool::Health,
            total: 112,
            ticks: 16 * rules::TICKS_PER_SECOND,
            range: 165,
            eats_a_tree: true,
            breaks: false,
        }),
        stock: Some(Stock {
            initial: 10,
            max: 10,
            restock_ticks: 120 * rules::TICKS_PER_SECOND,
        }),
        ..PLAIN
    },
    // Town Portal Scroll: Dota 7.41f `item_tpscroll` of the game's
    // `scripts/npc/items.txt` (client 6957, 2026-10-09), the official patch
    // notes 7.33 to 7.41f and Liquipedia's *Town Portal Scroll* (read
    // 2026-10-11). Stacks without limit in Dota; a stack's charges are a byte
    // here, more than a match buys.
    ItemDef {
        name: "Town Portal Scroll",
        cost: 100,
        charges: 1,
        stack_limit: u8::MAX,
        cooldown: rules::SCROLL_WAIT_TICKS,
        shared_wait: true,
        mana_cost: rules::SCROLL_MANA,
        aim: Some(Aim::Building),
        duration: rules::SCROLL_CHANNEL_TICKS,
        effect: UseSpec::Teleport,
        ..PLAIN
    },
    ItemDef {
        name: "Circlet",
        cost: 155,
        carried: Carried {
            attributes: Attributes::all(2),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Gauntlets of Strength",
        cost: 140,
        carried: Carried {
            attributes: points(Attribute::Strength, 3),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Slippers of Agility",
        cost: 140,
        carried: Carried {
            attributes: points(Attribute::Agility, 3),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Mantle of Intelligence",
        cost: 140,
        carried: Carried {
            attributes: points(Attribute::Intelligence, 3),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Belt of Strength",
        cost: 450,
        carried: Carried {
            attributes: points(Attribute::Strength, 6),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Band of Elvenskin",
        cost: 450,
        carried: Carried {
            attributes: points(Attribute::Agility, 6),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Robe of the Magi",
        cost: 450,
        carried: Carried {
            attributes: points(Attribute::Intelligence, 6),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Ogre Axe",
        cost: 1000,
        carried: Carried {
            attributes: points(Attribute::Strength, 10),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Blade of Alacrity",
        cost: 1000,
        carried: Carried {
            attributes: points(Attribute::Agility, 10),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Staff of Wizardry",
        cost: 1000,
        carried: Carried {
            attributes: points(Attribute::Intelligence, 10),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Gloves of Haste",
        cost: 450,
        carried: Carried {
            attack_speed: 20,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Blades of Attack",
        cost: 450,
        carried: Carried {
            damage: 9,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Broadsword",
        cost: 1000,
        carried: Carried {
            damage: 15,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Quarterstaff",
        cost: 875,
        carried: Carried {
            damage: 10,
            attack_speed: 10,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Ring of Protection",
        cost: 175,
        carried: Carried {
            armor: Fixed::from_int(2),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Chainmail",
        cost: 500,
        carried: Carried {
            armor: Fixed::from_int(4),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Ring of Regen",
        cost: 175,
        carried: Carried {
            hp_regen: hundredths_per_second(125),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Sage's Mask",
        cost: 175,
        carried: Carried {
            mana_regen: hundredths_per_second(70),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Vitality Booster",
        cost: 1000,
        carried: Carried { hp: 250, ..NOTHING },
        ..PLAIN
    },
    ItemDef {
        name: "Energy Booster",
        cost: 800,
        carried: Carried {
            mana: 250,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Power Treads",
        cost: 1400,
        mode: Some(Attribute::Strength),
        mode_bonus: 10,
        components: &TREADS_PARTS,
        alternatives: &TREADS_ALTERNATIVES,
        carried: Carried {
            move_speed: 55,
            attack_speed: 25,
            ..NOTHING
        },
        ranged: Some(Carried {
            move_speed: 45,
            attack_speed: 25,
            ..NOTHING
        }),
        aim: Some(Aim::Own),
        immediate: true,
        effect: UseSpec::SwitchMode,
        ..PLAIN
    },
    ItemDef {
        name: "Phase Boots",
        cost: 1450,
        cooldown: 240,
        components: &PHASE_PARTS,
        carried: Carried {
            move_speed: 50,
            damage: 18,
            armor: Fixed::from_int(4),
            ..NOTHING
        },
        ranged: Some(Carried {
            move_speed: 50,
            damage: 12,
            armor: Fixed::from_int(4),
            ..NOTHING
        }),
        aim: Some(Aim::Own),
        ignores_channel: true,
        effect: UseSpec::Phase {
            pct: 20,
            ranged_pct: 10,
            ticks: 3 * rules::TICKS_PER_SECOND,
        },
        ..PLAIN
    },
    ItemDef {
        name: "Blink Dagger",
        cost: 2250,
        cooldown: 450,
        breaks_on_damage: 90,
        aim: Some(Aim::Point),
        range: 1200,
        effect: UseSpec::Blink {
            range: BLINK_RANGE,
            overshoot: BLINK_OVERSHOOT_RANGE,
        },
        ..PLAIN
    },
    ItemDef {
        name: "Bracer",
        cost: 505,
        components: &BRACER_PARTS,
        carried: Carried {
            attributes: points_with(Attribute::Strength, 5, 2),
            hp: 50,
            hp_regen: hundredths_per_second(75),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Wraith Band",
        cost: 505,
        components: &WRAITH_PARTS,
        carried: Carried {
            attributes: points_with(Attribute::Agility, 5, 2),
            attack_speed: 6,
            armor: Fixed::from_ratio(7, 4),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Null Talisman",
        cost: 505,
        components: &NULL_PARTS,
        carried: Carried {
            attributes: points_with(Attribute::Intelligence, 5, 2),
            mana_regen: per_second(1),
            mana_pct: 3,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Magic Stick",
        cost: 200,
        cast_charges: 10,
        cooldown: 17 * rules::TICKS_PER_SECOND,
        aim: Some(Aim::Own),
        spends: Spends::All,
        effect: UseSpec::Restore {
            hp_per_charge: 15,
            mana_per_charge: 15,
        },
        ..PLAIN
    },
    ItemDef {
        name: "Magic Wand",
        cost: 460,
        cast_charges: 20,
        cooldown: 15 * rules::TICKS_PER_SECOND,
        components: &WAND_PARTS,
        carried: Carried {
            attributes: Attributes::all(3),
            ..NOTHING
        },
        aim: Some(Aim::Own),
        spends: Spends::All,
        effect: UseSpec::Restore {
            hp_per_charge: 15,
            mana_per_charge: 15,
        },
        ..PLAIN
    },
    // Retired: Phase Boots build of their parts alone since Dota made the
    // recipe free. The id keeps its place.
    ItemDef {
        name: "Recipe: Phase Boots",
        retired: true,
        ..PLAIN
    },
    ItemDef {
        name: "Recipe: Bracer",
        cost: 210,
        ..PLAIN
    },
    ItemDef {
        name: "Recipe: Wraith Band",
        cost: 210,
        ..PLAIN
    },
    ItemDef {
        name: "Recipe: Null Talisman",
        cost: 210,
        ..PLAIN
    },
    ItemDef {
        name: "Recipe: Magic Wand",
        cost: 150,
        ..PLAIN
    },
    ItemDef {
        name: "Enchanted Mango",
        cost: MANGO_COST,
        charges: 1,
        stack_limit: MANGO_STACK_MAX,
        carried: Carried {
            hp_regen: MANGO_HP_REGEN,
            ..NOTHING
        },
        aim: Some(Aim::Own),
        spends: Spends::One,
        mana_deficit: true,
        effect: UseSpec::Replenish { mana: MANGO_MANA },
        stock: Some(Stock {
            initial: 4,
            max: 4,
            restock_ticks: 120 * rules::TICKS_PER_SECOND,
        }),
        ..PLAIN
    },
    ItemDef {
        name: "Eaglesong",
        cost: 2800,
        carried: Carried {
            attributes: points(Attribute::Agility, 25),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Claymore",
        cost: 1350,
        carried: Carried {
            damage: 20,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Talisman of Evasion",
        cost: 1300,
        carried: Carried {
            evasion: Odds::percent(15),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Butterfly",
        cost: 5450,
        components: &BUTTERFLY_PARTS,
        carried: Carried {
            attributes: points(Attribute::Agility, 30),
            damage: 30,
            base_attack_speed_pct: 20,
            evasion: Odds::percent(35),
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Javelin",
        cost: 900,
        carried: Carried {
            pierce: Odds::percent(25),
            pierce_damage: 60,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Demon Edge",
        cost: 2200,
        carried: Carried {
            damage: 40,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Blitz Knuckles",
        cost: 1000,
        carried: Carried {
            attack_speed: 35,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Recipe: Monkey King Bar",
        cost: 900,
        ..PLAIN
    },
    ItemDef {
        name: "Monkey King Bar",
        cost: 5000,
        components: &MKB_PARTS,
        carried: Carried {
            damage: 50,
            attack_speed: 50,
            melee_range: 50,
            pierce: Odds::percent(80),
            pierce_damage: 70,
            pierce_unique: true,
            ..NOTHING
        },
        ..PLAIN
    },
    // Wind Lace and Blood Grenade: Dota 7.41f's official item data
    // (dota2.com datafeed, items 244 and 1123), the game's
    // `scripts/npc/items.txt` (client 6957, 2026-10-09) for stock and stacks,
    // and Liquipedia's *Blood Grenade* and *Movement Speed*.
    ItemDef {
        name: "Wind Lace",
        cost: 225,
        carried: Carried {
            lace_speed: 15,
            ..NOTHING
        },
        ..PLAIN
    },
    ItemDef {
        name: "Blood Grenade",
        cost: 50,
        charges: 1,
        stack_limit: 2,
        cooldown: GRENADE_COOLDOWN,
        health_cost: 75,
        stock: Some(Stock {
            initial: 2,
            max: 3,
            restock_ticks: 180 * rules::TICKS_PER_SECOND,
        }),
        carried: Carried { hp: 50, ..NOTHING },
        aim: Some(Aim::Point),
        range: GRENADE_RANGE,
        spends: Spends::One,
        point: 200,
        effect: UseSpec::Grenade,
        ..PLAIN
    },
    // Ward Dispenser: `item_ward_dispenser` and `item_recipe_ward_dispenser`
    // of the same file, and Liquipedia's *Observer and Sentry Wards*. It is
    // made of the wards a seat holds (one Observer and one Sentry Ward are the
    // recipe; the item keeps the count of each), is never sold, and plants the
    // kind it is switched to by the entry of that ward. The file gives the
    // dispenser no cooldown of its own, but Liquipedia's changelog (2017,
    // "Fixed Plant cooldown not working") has its plant wait, so it keeps the
    // second the wards wait between plantings.
    ItemDef {
        name: "Ward Dispenser",
        cost: 50,
        charges: 1,
        cooldown: 30,
        aim: Some(Aim::Point),
        range: 500,
        unbuyable: true,
        effect: UseSpec::Dispense,
        ..PLAIN
    },
];

const _: () = {
    // A stocked item is bought whole, and is no part of a build.
    let mut at = 0;
    while at < ITEMS.len() {
        let def = &ITEMS[at];
        let instant = def.point == 0 && def.backswing == 0 && def.duration == 0;
        assert!(!def.ignores_channel || instant);
        assert!(!def.immediate || instant);
        if let Some(stock) = ITEMS[at].stock {
            assert!(ITEMS[at].components.is_empty());
            assert!(stock.initial <= stock.max);
            assert!(stock.max > 0);
            assert!(stock.restock_ticks > 0);
        }
        let mut part = 0;
        while part < ITEMS[at].components.len() {
            let held = &ITEMS[ITEMS[at].components[part].0 as usize];
            assert!(held.stock.is_none());
            // Nothing is built of a retired entry.
            assert!(!held.retired);
            part += 1;
        }
        // Every other recipe costs what the first does, and only a built
        // item has one.
        let mut other = 0;
        while other < def.alternatives.len() {
            assert!(!def.components.is_empty());
            assert!(parts_cost(def.alternatives[other].parts) == parts_cost(def.components));
            other += 1;
        }
        // What the server relies on whatever a client orders: a recipe has
        // parts, within the slots an assembly takes, that a shop sells whole;
        // a stackable is bought in a lot of charges that fits its limit, with
        // no parts of its own; a cost is a price.
        assert!(def.components.len() <= MAX_COMPONENTS);
        assert!(def.cost >= 0);
        if def.stack_limit > 0 {
            assert!(def.charges >= 1 && def.charges <= def.stack_limit);
            assert!(def.components.is_empty() && def.alternatives.is_empty());
        }
        if let UseSpec::Replenish { mana } = def.effect {
            assert!(mana > 0);
        }
        let mut part = 0;
        while part < def.components.len() {
            assert!(sold_whole(def.components[part]));
            part += 1;
        }
        let mut other = 0;
        while other < def.alternatives.len() {
            let parts = def.alternatives[other].parts;
            assert!(!parts.is_empty() && parts.len() <= MAX_COMPONENTS);
            let mut part = 0;
            while part < parts.len() {
                assert!(sold_whole(parts[part]));
                part += 1;
            }
            other += 1;
        }
        // A retired entry costs, builds and carries nothing.
        if def.retired {
            assert!(def.cost == 0 && def.components.is_empty() && def.ranged.is_none());
        }
        at += 1;
    }
};

/// Whether a shop can sell an item as a part of a build: it is one of the
/// catalog's, a buyer takes it whole and no stock is kept of it.
const fn sold_whole(item: ItemId) -> bool {
    let def = &ITEMS[item.0 as usize];
    !def.unbuyable && def.stack_limit == 0
}

// The scroll every hero starts with is a one-charge purchase.
const _: () = {
    let scroll = &ITEMS[ItemId::TOWN_PORTAL_SCROLL.0 as usize];
    assert!(scroll.charges == 1 && !scroll.unbuyable);
};

/// What a set of parts costs together.
const fn parts_cost(parts: &[ItemId]) -> i32 {
    let mut sum = 0;
    let mut at = 0;
    while at < parts.len() {
        sum += ITEMS[parts[at].0 as usize].cost;
        at += 1;
    }
    sum
}
