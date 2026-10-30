//! What bota does with each item: its row of the catalog, built from what Dota's script gives
//! it (`items.rs`, by the item's key) and what that script cannot say: how a use is aimed and
//! what it does, and which of its values stand for which of bota's stats. Every number comes
//! from the script by its Dota key; a number written here is bota's own, and says why.

use bota_proto::{Aim, Attribute, Attributes, Fixed};

use super::items::*;
use crate::{
    Carried, Dec, ItemData, ItemDef, Mend, NOTHING, OBSERVER_WARD, Odds, PLAIN, Pool, Recipe,
    SENTRY_WARD, Spends, Stock, UseSpec, hundredths_per_second, points, points_with,
};

/// The row of an item as its script gives it; a row of the catalog adds what bota does with it.
const fn from(d: &ItemData) -> ItemDef {
    ItemDef {
        id: d.id,
        key: d.key,
        retired: d.retired,
        cost: d.cost,
        charges: d.charges,
        stack_limit: d.stack,
        cooldown: d.cooldown.ticks(),
        mana_cost: d.mana,
        health_cost: d.health,
        stock: match d.stock {
            Some(stock) => Some(Stock {
                initial: stock.initial,
                max: stock.max,
                restock_ticks: stock.every.ticks(),
            }),
            None => None,
        },
        cast_range: d.cast_range,
        behavior: d.behavior,
        values: d.values,
        point: d.cast_point.0,
        duration: d.channel.ticks(),
        unbuyable: !d.purchasable,
        components: if d.builds.is_empty() {
            &[]
        } else {
            d.builds[0]
        },
        ..PLAIN
    }
}

/// A value's first level as a whole number.
const fn int(d: &ItemData, key: &str) -> i32 {
    d.first(key).int()
}

/// A value's first level as a count a byte holds.
const fn byte(d: &ItemData, key: &str) -> u8 {
    let value = int(d, key);
    assert!(
        value >= 0 && value <= u8::MAX as i32,
        "more than a byte holds"
    );
    value as u8
}

/// A value's first level as a chance, in percent.
const fn odds(d: &ItemData, key: &str) -> Odds {
    let value = int(d, key);
    assert!(
        value >= 0 && value <= 100,
        "a chance past a hundred percent"
    );
    Odds::percent(value as u8)
}

/// A value's first level as a span in ticks.
const fn ticks(d: &ItemData, key: &str) -> u32 {
    d.first(key).secs().ticks()
}

/// A value's first level, regeneration a second, as a tick's worth.
const fn regen(d: &ItemData, key: &str) -> Fixed {
    hundredths_per_second(d.first(key).hundredths())
}

/// A value's first level as armor.
const fn armor(d: &ItemData, key: &str) -> Fixed {
    let value: Dec = d.first(key);
    Fixed::from_ratio(value.0, 1000)
}

/// A drink that mends `pool` by the value `per_second` for the value `duration`.
const fn mend(d: &ItemData, pool: Pool, per_second: &str, duration: &str, tree: bool) -> Mend {
    let per_second = d.first(per_second);
    let seconds = d.first(duration);
    Mend {
        pool,
        total: per_second.0 * seconds.0 / 1_000_000,
        ticks: seconds.secs().ticks(),
        eats_a_tree: tree,
        breaks: !tree,
    }
}

/// An item that adds one attribute's `bonus_*` value and nothing else.
const fn attribute(d: &ItemData, which: Attribute, key: &str) -> ItemDef {
    ItemDef {
        carried: Carried {
            attributes: points(which, int(d, key)),
            ..NOTHING
        },
        ..from(d)
    }
}

/// An item that adds its `bonus_damage` and nothing else.
const fn damage(d: &ItemData) -> ItemDef {
    ItemDef {
        carried: Carried {
            damage: int(d, "bonus_damage"),
            ..NOTHING
        },
        ..from(d)
    }
}

/// A stat item of the bracer's family: five of its attribute, two of the others.
const fn bracer_kin(d: &ItemData, which: Attribute, five: &str, two: &str) -> Attributes {
    points_with(which, int(d, five), int(d, two))
}

/// Every item, each its script's row and what bota does with it.
pub(crate) const ITEM_ROWS: &[ItemDef] = &[
    ItemDef {
        name: "Boots of Speed",
        carried: Carried {
            move_speed: int(&ITEM_BOOTS, "bonus_movement_speed"),
            ..NOTHING
        },
        ..from(&ITEM_BOOTS)
    },
    ItemDef {
        name: "Clarity",
        aim: Some(Aim::Unit),
        spends: Spends::One,
        effect: UseSpec::Mend(mend(
            &ITEM_CLARITY,
            Pool::Mana,
            "mana_regen",
            "buff_duration",
            false,
        )),
        ..from(&ITEM_CLARITY)
    },
    ItemDef {
        name: "Healing Salve",
        aim: Some(Aim::Unit),
        spends: Spends::One,
        effect: UseSpec::Mend(mend(
            &ITEM_FLASK,
            Pool::Health,
            "health_regen",
            "buff_duration",
            false,
        )),
        ..from(&ITEM_FLASK)
    },
    ItemDef {
        name: "Iron Branch",
        // One planting: the planted branch is gone.
        charges: 1,
        carried: Carried {
            attributes: Attributes::all(int(&ITEM_BRANCHES, "bonus_all_stats")),
            ..NOTHING
        },
        aim: Some(Aim::Point),
        spends: Spends::One,
        effect: UseSpec::PlantTree {
            ticks: ticks(&ITEM_BRANCHES, "tree_duration"),
        },
        ..from(&ITEM_BRANCHES)
    },
    ItemDef {
        name: "Observer Ward",
        aim: Some(Aim::Point),
        spends: Spends::One,
        effect: UseSpec::Ward {
            unit: &OBSERVER_WARD,
            ticks: ticks(&ITEM_WARD_OBSERVER, "lifetime"),
        },
        ..from(&ITEM_WARD_OBSERVER)
    },
    ItemDef {
        name: "Quelling Blade",
        carried: Carried {
            damage_to_creeps: int(&ITEM_QUELLING_BLADE, "damage_bonus"),
            ..NOTHING
        },
        ranged: Some(Carried {
            damage_to_creeps: int(&ITEM_QUELLING_BLADE, "damage_bonus_ranged"),
            ..NOTHING
        }),
        aim: Some(Aim::Tree),
        effect: UseSpec::FellTree,
        ..from(&ITEM_QUELLING_BLADE)
    },
    ItemDef {
        name: "Sentry Ward",
        aim: Some(Aim::Point),
        spends: Spends::One,
        effect: UseSpec::Ward {
            unit: &SENTRY_WARD,
            ticks: ticks(&ITEM_WARD_SENTRY, "lifetime"),
        },
        ..from(&ITEM_WARD_SENTRY)
    },
    ItemDef {
        name: "Tango",
        aim: Some(Aim::Tree),
        spends: Spends::One,
        effect: UseSpec::Mend(mend(
            &ITEM_TANGO,
            Pool::Health,
            "health_regen",
            "buff_duration",
            true,
        )),
        ..from(&ITEM_TANGO)
    },
    // Town Portal Scroll: also the official patch notes 7.33 to 7.41f and Liquipedia's *Town
    // Portal Scroll* (read 2026-10-11). A stack's charges are a byte here, more than a match buys.
    ItemDef {
        name: "Town Portal Scroll",
        shared_wait: true,
        aim: Some(Aim::Building),
        effect: UseSpec::Teleport,
        ..from(&ITEM_TPSCROLL)
    },
    ItemDef {
        name: "Circlet",
        carried: Carried {
            attributes: Attributes::all(int(&ITEM_CIRCLET, "bonus_all_stats")),
            ..NOTHING
        },
        ..from(&ITEM_CIRCLET)
    },
    ItemDef {
        name: "Gauntlets of Strength",
        ..attribute(&ITEM_GAUNTLETS, Attribute::Strength, "bonus_strength")
    },
    ItemDef {
        name: "Slippers of Agility",
        ..attribute(&ITEM_SLIPPERS, Attribute::Agility, "bonus_agility")
    },
    ItemDef {
        name: "Mantle of Intelligence",
        ..attribute(&ITEM_MANTLE, Attribute::Intelligence, "bonus_intellect")
    },
    ItemDef {
        name: "Belt of Strength",
        ..attribute(
            &ITEM_BELT_OF_STRENGTH,
            Attribute::Strength,
            "bonus_strength",
        )
    },
    ItemDef {
        name: "Band of Elvenskin",
        ..attribute(&ITEM_BOOTS_OF_ELVES, Attribute::Agility, "bonus_agility")
    },
    ItemDef {
        name: "Robe of the Magi",
        ..attribute(&ITEM_ROBE, Attribute::Intelligence, "bonus_intellect")
    },
    ItemDef {
        name: "Ogre Axe",
        ..attribute(&ITEM_OGRE_AXE, Attribute::Strength, "bonus_strength")
    },
    ItemDef {
        name: "Blade of Alacrity",
        ..attribute(&ITEM_BLADE_OF_ALACRITY, Attribute::Agility, "bonus_agility")
    },
    ItemDef {
        name: "Staff of Wizardry",
        ..attribute(
            &ITEM_STAFF_OF_WIZARDRY,
            Attribute::Intelligence,
            "bonus_intellect",
        )
    },
    ItemDef {
        name: "Gloves of Haste",
        carried: Carried {
            attack_speed: int(&ITEM_GLOVES, "bonus_attack_speed"),
            ..NOTHING
        },
        ..from(&ITEM_GLOVES)
    },
    ItemDef {
        name: "Blades of Attack",
        ..damage(&ITEM_BLADES_OF_ATTACK)
    },
    ItemDef {
        name: "Broadsword",
        ..damage(&ITEM_BROADSWORD)
    },
    ItemDef {
        name: "Quarterstaff",
        carried: Carried {
            damage: int(&ITEM_QUARTERSTAFF, "bonus_damage"),
            attack_speed: int(&ITEM_QUARTERSTAFF, "bonus_speed"),
            ..NOTHING
        },
        ..from(&ITEM_QUARTERSTAFF)
    },
    ItemDef {
        name: "Ring of Protection",
        carried: Carried {
            armor: armor(&ITEM_RING_OF_PROTECTION, "bonus_armor"),
            ..NOTHING
        },
        ..from(&ITEM_RING_OF_PROTECTION)
    },
    ItemDef {
        name: "Chainmail",
        carried: Carried {
            armor: armor(&ITEM_CHAINMAIL, "bonus_armor"),
            ..NOTHING
        },
        ..from(&ITEM_CHAINMAIL)
    },
    ItemDef {
        name: "Ring of Regen",
        carried: Carried {
            hp_regen: regen(&ITEM_RING_OF_REGEN, "bonus_health_regen"),
            ..NOTHING
        },
        ..from(&ITEM_RING_OF_REGEN)
    },
    ItemDef {
        name: "Sage's Mask",
        carried: Carried {
            mana_regen: regen(&ITEM_SOBI_MASK, "bonus_mana_regen"),
            ..NOTHING
        },
        ..from(&ITEM_SOBI_MASK)
    },
    ItemDef {
        name: "Vitality Booster",
        carried: Carried {
            hp: int(&ITEM_VITALITY_BOOSTER, "bonus_health"),
            ..NOTHING
        },
        ..from(&ITEM_VITALITY_BOOSTER)
    },
    ItemDef {
        name: "Energy Booster",
        carried: Carried {
            mana: int(&ITEM_ENERGY_BOOSTER, "bonus_mana"),
            ..NOTHING
        },
        ..from(&ITEM_ENERGY_BOOSTER)
    },
    // Power Treads are built three ways, `item_recipe_power_treads` requirements 01, 04 and 06:
    // a Belt sets them to strength, a Robe to intelligence, a Band to agility.
    ItemDef {
        name: "Power Treads",
        mode: Some(Attribute::Strength),
        mode_bonus: int(&ITEM_POWER_TREADS, "bonus_stat"),
        alternatives: &[
            Recipe {
                parts: ITEM_POWER_TREADS.builds[1],
                mode: Some(Attribute::Intelligence),
            },
            Recipe {
                parts: ITEM_POWER_TREADS.builds[2],
                mode: Some(Attribute::Agility),
            },
        ],
        carried: Carried {
            move_speed: int(&ITEM_POWER_TREADS, "bonus_movement_speed_melee"),
            attack_speed: int(&ITEM_POWER_TREADS, "bonus_attack_speed"),
            ..NOTHING
        },
        ranged: Some(Carried {
            move_speed: int(&ITEM_POWER_TREADS, "bonus_movement_speed_ranged"),
            attack_speed: int(&ITEM_POWER_TREADS, "bonus_attack_speed"),
            ..NOTHING
        }),
        aim: Some(Aim::Own),
        immediate: true,
        effect: UseSpec::SwitchMode,
        ..from(&ITEM_POWER_TREADS)
    },
    ItemDef {
        name: "Phase Boots",
        carried: Carried {
            move_speed: int(&ITEM_PHASE_BOOTS, "bonus_movement_speed"),
            damage: int(&ITEM_PHASE_BOOTS, "bonus_damage_melee"),
            armor: armor(&ITEM_PHASE_BOOTS, "bonus_armor"),
            ..NOTHING
        },
        ranged: Some(Carried {
            move_speed: int(&ITEM_PHASE_BOOTS, "bonus_movement_speed"),
            damage: int(&ITEM_PHASE_BOOTS, "bonus_damage_range"),
            armor: armor(&ITEM_PHASE_BOOTS, "bonus_armor"),
            ..NOTHING
        }),
        aim: Some(Aim::Own),
        ignores_channel: true,
        effect: UseSpec::Phase {
            pct: int(&ITEM_PHASE_BOOTS, "phase_movement_speed"),
            ranged_pct: int(&ITEM_PHASE_BOOTS, "phase_movement_speed_range"),
            ticks: ticks(&ITEM_PHASE_BOOTS, "phase_duration"),
        },
        ..from(&ITEM_PHASE_BOOTS)
    },
    ItemDef {
        name: "Blink Dagger",
        breaks_on_damage: ticks(&ITEM_BLINK, "blink_damage_cooldown"),
        aim: Some(Aim::Point),
        effect: UseSpec::Blink {
            range: int(&ITEM_BLINK, "blink_range"),
            overshoot: int(&ITEM_BLINK, "blink_range_clamp"),
        },
        ..from(&ITEM_BLINK)
    },
    ItemDef {
        name: "Bracer",
        carried: Carried {
            attributes: bracer_kin(
                &ITEM_BRACER,
                Attribute::Strength,
                "bonus_strength",
                "bonus_agility",
            ),
            hp: int(&ITEM_BRACER, "bonus_health"),
            hp_regen: regen(&ITEM_BRACER, "bonus_health_regen"),
            ..NOTHING
        },
        ..from(&ITEM_BRACER)
    },
    ItemDef {
        name: "Wraith Band",
        carried: Carried {
            attributes: bracer_kin(
                &ITEM_WRAITH_BAND,
                Attribute::Agility,
                "bonus_agility",
                "bonus_strength",
            ),
            attack_speed: int(&ITEM_WRAITH_BAND, "bonus_attack_speed"),
            armor: armor(&ITEM_WRAITH_BAND, "bonus_armor"),
            ..NOTHING
        },
        ..from(&ITEM_WRAITH_BAND)
    },
    ItemDef {
        name: "Null Talisman",
        carried: Carried {
            attributes: bracer_kin(
                &ITEM_NULL_TALISMAN,
                Attribute::Intelligence,
                "bonus_intellect",
                "bonus_strength",
            ),
            mana_regen: regen(&ITEM_NULL_TALISMAN, "bonus_mana_regen"),
            mana_pct: int(&ITEM_NULL_TALISMAN, "bonus_max_mana_percentage"),
            ..NOTHING
        },
        ..from(&ITEM_NULL_TALISMAN)
    },
    ItemDef {
        name: "Magic Stick",
        cast_charges: byte(&ITEM_MAGIC_STICK, "max_charges"),
        aim: Some(Aim::Own),
        spends: Spends::All,
        effect: UseSpec::Restore {
            hp_per_charge: int(&ITEM_MAGIC_STICK, "restore_per_charge"),
            mana_per_charge: int(&ITEM_MAGIC_STICK, "restore_per_charge"),
        },
        ..from(&ITEM_MAGIC_STICK)
    },
    ItemDef {
        name: "Magic Wand",
        cast_charges: byte(&ITEM_MAGIC_WAND, "max_charges"),
        carried: Carried {
            attributes: Attributes::all(int(&ITEM_MAGIC_WAND, "bonus_all_stats")),
            ..NOTHING
        },
        aim: Some(Aim::Own),
        spends: Spends::All,
        effect: UseSpec::Restore {
            hp_per_charge: int(&ITEM_MAGIC_WAND, "restore_per_charge"),
            mana_per_charge: int(&ITEM_MAGIC_WAND, "restore_per_charge"),
        },
        ..from(&ITEM_MAGIC_WAND)
    },
    // Retired: Phase Boots build of their parts alone since Dota made the recipe free. The id
    // keeps its place.
    ItemDef {
        name: "Recipe: Phase Boots",
        ..from(&ITEM_RECIPE_PHASE_BOOTS)
    },
    ItemDef {
        name: "Recipe: Bracer",
        ..from(&ITEM_RECIPE_BRACER)
    },
    ItemDef {
        name: "Recipe: Wraith Band",
        ..from(&ITEM_RECIPE_WRAITH_BAND)
    },
    ItemDef {
        name: "Recipe: Null Talisman",
        ..from(&ITEM_RECIPE_NULL_TALISMAN)
    },
    ItemDef {
        name: "Recipe: Magic Wand",
        ..from(&ITEM_RECIPE_MAGIC_WAND)
    },
    ItemDef {
        name: "Enchanted Mango",
        carried: Carried {
            hp_regen: regen(&ITEM_ENCHANTED_MANGO, "hp_regen"),
            ..NOTHING
        },
        aim: Some(Aim::Own),
        // Eaten by its carrier only: no reach of its own.
        cast_range: 0,
        spends: Spends::One,
        mana_deficit: true,
        effect: UseSpec::Replenish {
            mana: int(&ITEM_ENCHANTED_MANGO, "replenish_amount"),
        },
        ..from(&ITEM_ENCHANTED_MANGO)
    },
    ItemDef {
        name: "Eaglesong",
        ..attribute(&ITEM_EAGLE, Attribute::Agility, "bonus_agility")
    },
    ItemDef {
        name: "Claymore",
        ..damage(&ITEM_CLAYMORE)
    },
    ItemDef {
        name: "Talisman of Evasion",
        carried: Carried {
            evasion: odds(&ITEM_TALISMAN_OF_EVASION, "bonus_evasion"),
            ..NOTHING
        },
        ..from(&ITEM_TALISMAN_OF_EVASION)
    },
    ItemDef {
        name: "Butterfly",
        carried: Carried {
            attributes: points(Attribute::Agility, int(&ITEM_BUTTERFLY, "bonus_agility")),
            damage: int(&ITEM_BUTTERFLY, "bonus_damage"),
            base_attack_speed_pct: int(&ITEM_BUTTERFLY, "bonus_attack_speed_pct"),
            evasion: odds(&ITEM_BUTTERFLY, "bonus_evasion"),
            ..NOTHING
        },
        ..from(&ITEM_BUTTERFLY)
    },
    ItemDef {
        name: "Javelin",
        carried: Carried {
            pierce: odds(&ITEM_JAVELIN, "bonus_chance"),
            pierce_damage: int(&ITEM_JAVELIN, "bonus_chance_damage"),
            ..NOTHING
        },
        ..from(&ITEM_JAVELIN)
    },
    ItemDef {
        name: "Demon Edge",
        ..damage(&ITEM_DEMON_EDGE)
    },
    ItemDef {
        name: "Blitz Knuckles",
        carried: Carried {
            attack_speed: int(&ITEM_BLITZ_KNUCKLES, "bonus_attack_speed"),
            ..NOTHING
        },
        ..from(&ITEM_BLITZ_KNUCKLES)
    },
    ItemDef {
        name: "Recipe: Monkey King Bar",
        ..from(&ITEM_RECIPE_MONKEY_KING_BAR)
    },
    ItemDef {
        name: "Monkey King Bar",
        carried: Carried {
            damage: int(&ITEM_MONKEY_KING_BAR, "bonus_damage"),
            attack_speed: int(&ITEM_MONKEY_KING_BAR, "bonus_attack_speed"),
            melee_range: int(&ITEM_MONKEY_KING_BAR, "melee_attack_range"),
            pierce: odds(&ITEM_MONKEY_KING_BAR, "bonus_chance"),
            pierce_damage: int(&ITEM_MONKEY_KING_BAR, "bonus_chance_damage"),
            pierce_unique: true,
            ..NOTHING
        },
        ..from(&ITEM_MONKEY_KING_BAR)
    },
    // Wind Lace and Blood Grenade: also Liquipedia's *Blood Grenade* and *Movement Speed*.
    ItemDef {
        name: "Wind Lace",
        carried: Carried {
            lace_speed: int(&ITEM_WIND_LACE, "movement_speed"),
            ..NOTHING
        },
        ..from(&ITEM_WIND_LACE)
    },
    ItemDef {
        name: "Blood Grenade",
        carried: Carried {
            hp: int(&ITEM_BLOOD_GRENADE, "bonus_health"),
            ..NOTHING
        },
        aim: Some(Aim::Point),
        spends: Spends::One,
        effect: UseSpec::Grenade,
        ..from(&ITEM_BLOOD_GRENADE)
    },
    // Ward Dispenser: also Liquipedia's *Observer and Sentry Wards*. It is made of the wards a
    // seat holds (one Observer and one Sentry Ward are the recipe; the item keeps the count of
    // each), is never sold, and plants the kind it is switched to by the entry of that ward.
    ItemDef {
        name: "Ward Dispenser",
        charges: 1,
        components: &[],
        aim: Some(Aim::Point),
        effect: UseSpec::Dispense,
        ..from(&ITEM_WARD_DISPENSER)
    },
];
