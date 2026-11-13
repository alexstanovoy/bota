//! What bota does with each item: its row of the catalog, built from what Dota's script gives
//! it (`items.rs`, by the item's key) and what that script cannot say: how a use is aimed and
//! what it does, and which of its values stand for which of bota's stats. Every number comes
//! from the script by its Dota key; a number written here is bota's own, and says why.

use bota_proto::{Aim, Attribute, Attributes, EffectId, Fixed, ItemId};

use super::items::*;
use crate::{
    Affects, AllyUse, Carried, Dec, DisassembleRule, Dispel, Drink, Grenade, Immunity, ItemData,
    ItemDef, LOTUS_TAKE_RANGE, MagicBlock, Mend, NOTHING, Odds, OnAttack, PLAIN, Pool, Recipe,
    Scroll, Spends, Stock, UseSpec, hundredths_per_second, points, points_with, rules,
};

/// The row of an item as its script gives it; a row of the catalog adds what bota does with it.
const fn from(d: &ItemData) -> ItemDef {
    ItemDef {
        id: d.id,
        key: d.key,
        retired: d.retired,
        cost: d.cost,
        secret_shop: d.secret_shop,
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
                opens_ticks: stock.opens.ticks(),
            }),
            None => None,
        },
        cast_range: d.cast_range,
        behavior: d.behavior,
        values: d.values,
        point: d.cast_point.0,
        duration: d.channel.ticks(),
        unbuyable: !d.purchasable,
        wait_group: d.shared_cooldown,
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

/// A value's first level, a percent of magic resistance, in basis points.
const fn magic_resist(d: &ItemData, key: &str) -> i32 {
    let value = int(d, key);
    assert!(
        value >= 0 && value < 100,
        "a magic resistance past all of it"
    );
    value * rules::NOMINAL_BP / 100
}

/// A value's first level, armor written as a reduction (`-2`), as the armor it takes off.
const fn armor_taken(d: &ItemData, key: &str) -> Fixed {
    let value: Dec = d.first(key);
    assert!(value.0 < 0, "an armor reduction is written below zero");
    Fixed::from_ratio(-value.0, 1000)
}

/// Whether what the item does to enemies goes through spell immunity: its `SpellImmunityType`.
const fn pierces_immunity(d: &ItemData) -> bool {
    matches!(d.immunity, Immunity::Pierced)
}

/// A value's first level, a percent, in basis points of [`rules::NOMINAL_BP`].
const fn bp(d: &ItemData, key: &str) -> i32 {
    let value = int(d, key);
    assert!(value >= 0 && value < 100, "a share past all of it");
    value * rules::NOMINAL_BP / 100
}

/// A value's first level, a percent with up to two decimals, in basis points.
const fn hundredths_bp(d: &ItemData, key: &str) -> i32 {
    d.first(key).hundredths()
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
pub(crate) static ITEM_ROWS: &[ItemDef] = &[
    ItemDef {
        carried: Carried {
            move_speed: int(&ITEM_BOOTS, "bonus_movement_speed"),
            ..NOTHING
        },
        ..from(&ITEM_BOOTS)
    },
    ItemDef {
        aim: Some(Aim::Unit),
        spends: Spends::One,
        effect: UseSpec::Mend(mend(
            &ITEM_CLARITY,
            Pool::Mana,
            "mana_regen",
            "buff_duration",
            false,
        )),
        // Dota's `ShowGiveIndicatorOnTargetCast` and its unit target: cast on an allied hero the drink is the ally's.
        ally: Some(AllyUse::Effect { percent: 100 }),
        ..from(&ITEM_CLARITY)
    },
    ItemDef {
        aim: Some(Aim::Unit),
        spends: Spends::One,
        effect: UseSpec::Mend(mend(
            &ITEM_FLASK,
            Pool::Health,
            "health_regen",
            "buff_duration",
            false,
        )),
        // "Heals for half the amount per second when cast on an ally" (the item's tooltip): bota's own 50.
        ally: Some(AllyUse::Effect { percent: 50 }),
        ..from(&ITEM_FLASK)
    },
    ItemDef {
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
        aim: Some(Aim::Point),
        spends: Spends::One,
        effect: UseSpec::Ward {
            units: &crate::OBSERVER_WARDS,
            ticks: ticks(&ITEM_WARD_OBSERVER, "lifetime"),
        },
        // "Hold Control to give one Observer Ward to an allied hero".
        ally: Some(AllyUse::Give { item: None }),
        ..from(&ITEM_WARD_OBSERVER)
    },
    ItemDef {
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
        aim: Some(Aim::Point),
        spends: Spends::One,
        effect: UseSpec::Ward {
            units: &crate::SENTRY_WARDS,
            ticks: ticks(&ITEM_WARD_SENTRY, "lifetime"),
        },
        // "Hold Control to give one Sentry Ward to an allied hero".
        ally: Some(AllyUse::Give { item: None }),
        ..from(&ITEM_WARD_SENTRY)
    },
    ItemDef {
        aim: Some(Aim::Tree),
        spends: Spends::One,
        effect: UseSpec::Mend(mend(
            &ITEM_TANGO,
            Pool::Health,
            "health_regen",
            "buff_duration",
            true,
        )),
        // Cast on an allied hero it "gives them one Tango": a Tango (Shared).
        ally: Some(AllyUse::Give {
            item: Some(ItemId::TANGO_SHARED),
        }),
        ..from(&ITEM_TANGO)
    },
    // Town Portal Scroll: also the official patch notes 7.33 to 7.41f and Liquipedia's *Town
    // Portal Scroll* (read 2026-10-11). A stack's charges are a byte here, more than a match buys.
    ItemDef {
        shared_wait: true,
        aim: Some(Aim::Building),
        effect: UseSpec::Teleport(Scroll {
            min_distance: int(&ITEM_TPSCROLL, "minimum_distance"),
            max_distance: int(&ITEM_TPSCROLL, "maximum_distance"),
            sight_radius: int(&ITEM_TPSCROLL, "vision_radius"),
        }),
        ..from(&ITEM_TPSCROLL)
    },
    ItemDef {
        carried: Carried {
            attributes: Attributes::all(int(&ITEM_CIRCLET, "bonus_all_stats")),
            ..NOTHING
        },
        ..from(&ITEM_CIRCLET)
    },
    ItemDef {
        ..attribute(&ITEM_GAUNTLETS, Attribute::Strength, "bonus_strength")
    },
    ItemDef {
        ..attribute(&ITEM_SLIPPERS, Attribute::Agility, "bonus_agility")
    },
    ItemDef {
        ..attribute(&ITEM_MANTLE, Attribute::Intelligence, "bonus_intellect")
    },
    ItemDef {
        ..attribute(
            &ITEM_BELT_OF_STRENGTH,
            Attribute::Strength,
            "bonus_strength",
        )
    },
    ItemDef {
        ..attribute(&ITEM_BOOTS_OF_ELVES, Attribute::Agility, "bonus_agility")
    },
    ItemDef {
        ..attribute(&ITEM_ROBE, Attribute::Intelligence, "bonus_intellect")
    },
    ItemDef {
        ..attribute(&ITEM_OGRE_AXE, Attribute::Strength, "bonus_strength")
    },
    ItemDef {
        ..attribute(&ITEM_BLADE_OF_ALACRITY, Attribute::Agility, "bonus_agility")
    },
    ItemDef {
        ..attribute(
            &ITEM_STAFF_OF_WIZARDRY,
            Attribute::Intelligence,
            "bonus_intellect",
        )
    },
    ItemDef {
        carried: Carried {
            attack_speed: int(&ITEM_GLOVES, "bonus_attack_speed"),
            ..NOTHING
        },
        ..from(&ITEM_GLOVES)
    },
    ItemDef {
        ..damage(&ITEM_BLADES_OF_ATTACK)
    },
    ItemDef {
        ..damage(&ITEM_BROADSWORD)
    },
    ItemDef {
        carried: Carried {
            damage: int(&ITEM_QUARTERSTAFF, "bonus_damage"),
            attack_speed: int(&ITEM_QUARTERSTAFF, "bonus_speed"),
            ..NOTHING
        },
        ..from(&ITEM_QUARTERSTAFF)
    },
    ItemDef {
        carried: Carried {
            armor: armor(&ITEM_RING_OF_PROTECTION, "bonus_armor"),
            ..NOTHING
        },
        ..from(&ITEM_RING_OF_PROTECTION)
    },
    ItemDef {
        carried: Carried {
            armor: armor(&ITEM_CHAINMAIL, "bonus_armor"),
            ..NOTHING
        },
        ..from(&ITEM_CHAINMAIL)
    },
    ItemDef {
        carried: Carried {
            hp_regen: regen(&ITEM_RING_OF_REGEN, "bonus_health_regen"),
            ..NOTHING
        },
        ..from(&ITEM_RING_OF_REGEN)
    },
    ItemDef {
        carried: Carried {
            mana_regen: regen(&ITEM_SOBI_MASK, "bonus_mana_regen"),
            ..NOTHING
        },
        ..from(&ITEM_SOBI_MASK)
    },
    ItemDef {
        carried: Carried {
            hp: int(&ITEM_VITALITY_BOOSTER, "bonus_health"),
            ..NOTHING
        },
        ..from(&ITEM_VITALITY_BOOSTER)
    },
    ItemDef {
        carried: Carried {
            mana: int(&ITEM_ENERGY_BOOSTER, "bonus_mana"),
            ..NOTHING
        },
        ..from(&ITEM_ENERGY_BOOSTER)
    },
    // Power Treads are built three ways, `item_recipe_power_treads` requirements 01, 04 and 06:
    // a Belt sets them to strength, a Robe to intelligence, a Band to agility.
    ItemDef {
        mode: Some(Attribute::Strength),
        mode_bonus: int(&ITEM_POWER_TREADS, "bonus_stat"),
        alternatives: &[
            Recipe {
                parts: ITEM_POWER_TREADS.builds[1],
                by_charges: false,
                mode: Some(Attribute::Intelligence),
            },
            Recipe {
                parts: ITEM_POWER_TREADS.builds[2],
                by_charges: false,
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
        breaks_on_damage: ticks(&ITEM_BLINK, "blink_damage_cooldown"),
        aim: Some(Aim::Point),
        effect: UseSpec::Blink {
            range: int(&ITEM_BLINK, "blink_range"),
            overshoot: int(&ITEM_BLINK, "blink_range_clamp"),
        },
        ..from(&ITEM_BLINK)
    },
    ItemDef {
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
        cast_charges: byte(&ITEM_MAGIC_STICK, "max_charges"),
        aim: Some(Aim::Own),
        spends: Spends::All,
        effect: UseSpec::Restore {
            hp_per_charge: int(&ITEM_MAGIC_STICK, "restore_per_charge"),
            mana_per_charge: int(&ITEM_MAGIC_STICK, "restore_per_charge"),
            charge_radius: int(&ITEM_MAGIC_STICK, "charge_radius"),
        },
        ..from(&ITEM_MAGIC_STICK)
    },
    ItemDef {
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
            charge_radius: int(&ITEM_MAGIC_WAND, "charge_radius"),
        },
        ..from(&ITEM_MAGIC_WAND)
    },
    // Retired: Phase Boots build of their parts alone since Dota made the recipe free. The id
    // keeps its place.
    ItemDef {
        ..from(&ITEM_RECIPE_PHASE_BOOTS)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_BRACER)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_WRAITH_BAND)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_NULL_TALISMAN)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_MAGIC_WAND)
    },
    ItemDef {
        carried: Carried {
            hp_regen: regen(&ITEM_ENCHANTED_MANGO, "hp_regen"),
            ..NOTHING
        },
        aim: Some(Aim::Own),
        spends: Spends::One,
        mana_deficit: true,
        // "Hold Control to use on a nearby allied hero", within the script's 400.
        ally: Some(AllyUse::Effect { percent: 100 }),
        effect: UseSpec::Replenish {
            mana: int(&ITEM_ENCHANTED_MANGO, "replenish_amount"),
        },
        ..from(&ITEM_ENCHANTED_MANGO)
    },
    ItemDef {
        ..attribute(&ITEM_EAGLE, Attribute::Agility, "bonus_agility")
    },
    ItemDef {
        ..damage(&ITEM_CLAYMORE)
    },
    ItemDef {
        carried: Carried {
            evasion: odds(&ITEM_TALISMAN_OF_EVASION, "bonus_evasion"),
            ..NOTHING
        },
        ..from(&ITEM_TALISMAN_OF_EVASION)
    },
    ItemDef {
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
        carried: Carried {
            pierce: odds(&ITEM_JAVELIN, "bonus_chance"),
            pierce_damage: int(&ITEM_JAVELIN, "bonus_chance_damage"),
            ..NOTHING
        },
        ..from(&ITEM_JAVELIN)
    },
    ItemDef {
        ..damage(&ITEM_DEMON_EDGE)
    },
    ItemDef {
        carried: Carried {
            attack_speed: int(&ITEM_BLITZ_KNUCKLES, "bonus_attack_speed"),
            ..NOTHING
        },
        ..from(&ITEM_BLITZ_KNUCKLES)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_MONKEY_KING_BAR)
    },
    ItemDef {
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
        carried: Carried {
            lace_speed: int(&ITEM_WIND_LACE, "movement_speed"),
            ..NOTHING
        },
        ..from(&ITEM_WIND_LACE)
    },
    ItemDef {
        carried: Carried {
            hp: int(&ITEM_BLOOD_GRENADE, "bonus_health"),
            ..NOTHING
        },
        aim: Some(Aim::Point),
        spends: Spends::One,
        effect: UseSpec::Grenade(Grenade {
            speed: int(&ITEM_BLOOD_GRENADE, "speed"),
            radius: int(&ITEM_BLOOD_GRENADE, "radius"),
            impact_damage: int(&ITEM_BLOOD_GRENADE, "impact_damage"),
            damage_per_second: int(&ITEM_BLOOD_GRENADE, "damage_over_time"),
            slow_pct: -int(&ITEM_BLOOD_GRENADE, "movespeed_slow"),
            // And the tick of its last beat, so that beat lands before it lifts.
            debuff_ticks: ticks(&ITEM_BLOOD_GRENADE, "debuff_duration") + 1,
        }),
        ..from(&ITEM_BLOOD_GRENADE)
    },
    // Ward Dispenser: also Liquipedia's *Observer and Sentry Wards*. It is made of the wards a
    // seat holds (one Observer and one Sentry Ward are the recipe; the item keeps the count of
    // each), is never sold, and plants the kind it is switched to by the entry of that ward.
    ItemDef {
        charges: 1,
        components: &[],
        aim: Some(Aim::Point),
        effect: UseSpec::Dispense,
        // "Hold Control to give one ward to an allied hero": the kind it is switched to.
        ally: Some(AllyUse::Give { item: None }),
        disassemble: DisassembleRule::Never,
        ..from(&ITEM_WARD_DISPENSER)
    },
    // Tango (Shared): `item_tango_single`, the Tango a Tango cast on an allied hero leaves. Its
    // 3.5 health a second is 7.40's (Liquipedia's *Tango*), and it is gone `duration_to_use`
    // after it is made. Never bought, never sold.
    ItemDef {
        aim: Some(Aim::Tree),
        spends: Spends::One,
        effect: UseSpec::Mend(mend(
            &ITEM_TANGO_SINGLE,
            Pool::Health,
            "health_regen",
            "buff_duration",
            true,
        )),
        lifetime: ticks(&ITEM_TANGO_SINGLE, "duration_to_use"),
        ..from(&ITEM_TANGO_SINGLE)
    },
    // Bottle: also Liquipedia's *Bottle* and the rune rules of `rune.rs`. Three charges kept (an
    // empty one stays, to be filled at the fountain), used beside whatever the body is doing, on
    // its user or an allied hero in reach; it keeps a rune its holder takes.
    ItemDef {
        aim: Some(Aim::Own),
        spends: Spends::One,
        immediate: true,
        kept_empty: true,
        ally_target: true,
        effect: UseSpec::Bottle(Drink {
            health: int(&ITEM_BOTTLE, "health_restore"),
            mana: int(&ITEM_BOTTLE, "mana_restore"),
            ticks: ticks(&ITEM_BOTTLE, "restore_time"),
            range: ITEM_BOTTLE.cast_range,
        }),
        ..from(&ITEM_BOTTLE)
    },
    // The lotuses: also Liquipedia's *Healing Lotus* (`lotus.rs` names the sources). A Lotus Pool
    // grows them; none is sold. Eaten at once beside whatever the body is doing, one charge each,
    // on the user or on an allied hero in reach, one wait of five seconds shared by the three
    // kinds. Each is built by charges as the script's recipes say (`ItemRecipeConsumesCharges`):
    // three Healing Lotuses make a Great one, two Great ones a Greater one.
    lotus(&ITEM_FAMANGO),
    lotus(&ITEM_GREAT_FAMANGO),
    lotus(&ITEM_GREATER_FAMANGO),
    // Cheese: eaten at once by its holder alone. Roshan drops it; where it comes from is not
    // this table's.
    ItemDef {
        aim: Some(Aim::Own),
        spends: Spends::One,
        immediate: true,
        effect: UseSpec::Eat {
            health: int(&ITEM_CHEESE, "health_restore"),
            mana: int(&ITEM_CHEESE, "mana_restore"),
        },
        ..from(&ITEM_CHEESE)
    },
    // Royale with Cheese (shown as Block of Cheese): built, with no recipe scroll, when a bag holds
    // three Greater Healing Lotuses and a Cheese (Liquipedia, *Block of Cheese*, since 7.33; the
    // script has no recipe for it). Eaten at once by its holder alone. Its damage barrier (shield
    // 500, regeneration 100 a second) is not modelled yet.
    ItemDef {
        aim: Some(Aim::Own),
        spends: Spends::One,
        immediate: true,
        charge_recipes: &[&[
            ItemId::GREATER_HEALING_LOTUS,
            ItemId::GREATER_HEALING_LOTUS,
            ItemId::GREATER_HEALING_LOTUS,
            ItemId::CHEESE,
        ]],
        effect: UseSpec::Eat {
            health: int(&ITEM_ROYALE_WITH_CHEESE, "health_restore"),
            mana: int(&ITEM_ROYALE_WITH_CHEESE, "mana_restore"),
        },
        ..from(&ITEM_ROYALE_WITH_CHEESE)
    },
    // The basic shop's stat items: what each adds and nothing else.
    ItemDef {
        carried: Carried {
            attributes: Attributes::all(int(&ITEM_CROWN, "bonus_all_stats")),
            ..NOTHING
        },
        ..from(&ITEM_CROWN)
    },
    ItemDef {
        carried: Carried {
            attributes: Attributes::all(int(&ITEM_DIADEM, "bonus_all_stats")),
            ..NOTHING
        },
        ..from(&ITEM_DIADEM)
    },
    ItemDef {
        carried: Carried {
            armor: armor(&ITEM_SPLINTMAIL, "bonus_armor"),
            ..NOTHING
        },
        ..from(&ITEM_SPLINTMAIL)
    },
    ItemDef {
        carried: Carried {
            armor: armor(&ITEM_HELM_OF_IRON_WILL, "bonus_armor"),
            hp_regen: regen(&ITEM_HELM_OF_IRON_WILL, "bonus_regen"),
            ..NOTHING
        },
        ..from(&ITEM_HELM_OF_IRON_WILL)
    },
    ItemDef {
        ..damage(&ITEM_MITHRIL_HAMMER)
    },
    ItemDef {
        carried: Carried {
            hp: int(&ITEM_FLUFFY_HAT, "bonus_health"),
            ..NOTHING
        },
        ..from(&ITEM_FLUFFY_HAT)
    },
    ItemDef {
        carried: Carried {
            mana: int(&ITEM_WIZARD_HAT, "bonus_mana"),
            ..NOTHING
        },
        ..from(&ITEM_WIZARD_HAT)
    },
    ItemDef {
        carried: Carried {
            hp_regen: regen(&ITEM_RING_OF_HEALTH, "bonus_health_regen"),
            ..NOTHING
        },
        ..from(&ITEM_RING_OF_HEALTH)
    },
    ItemDef {
        carried: Carried {
            mana_regen: regen(&ITEM_VOID_STONE, "bonus_mana_regen"),
            ..NOTHING
        },
        ..from(&ITEM_VOID_STONE)
    },
    ItemDef {
        carried: Carried {
            armor: armor(&ITEM_PLATEMAIL, "bonus_armor"),
            ..NOTHING
        },
        ..from(&ITEM_PLATEMAIL)
    },
    ItemDef {
        carried: Carried {
            hp_regen: regen(&ITEM_RING_OF_TARRASQUE, "bonus_health_regen"),
            ..NOTHING
        },
        ..from(&ITEM_RING_OF_TARRASQUE)
    },
    ItemDef {
        carried: Carried {
            mana_regen: regen(&ITEM_TIARA_OF_SELEMENE, "bonus_mana_regen"),
            ..NOTHING
        },
        ..from(&ITEM_TIARA_OF_SELEMENE)
    },
    ItemDef {
        carried: Carried {
            attack_speed: int(&ITEM_HYPERSTONE, "bonus_attack_speed"),
            ..NOTHING
        },
        ..from(&ITEM_HYPERSTONE)
    },
    ItemDef {
        carried: Carried {
            hp: int(&ITEM_POINT_BOOSTER, "bonus_health"),
            mana: int(&ITEM_POINT_BOOSTER, "bonus_mana"),
            ..NOTHING
        },
        ..from(&ITEM_POINT_BOOSTER)
    },
    ItemDef {
        ..damage(&ITEM_RELIC)
    },
    ItemDef {
        ..attribute(&ITEM_REAVER, Attribute::Strength, "bonus_strength")
    },
    ItemDef {
        ..attribute(
            &ITEM_MYSTIC_STAFF,
            Attribute::Intelligence,
            "bonus_intellect",
        )
    },
    ItemDef {
        carried: Carried {
            attributes: Attributes::all(int(&ITEM_ULTIMATE_ORB, "bonus_all_stats")),
            ..NOTHING
        },
        ..from(&ITEM_ULTIMATE_ORB)
    },
    // Shawl and Cloak: magic resistance, stacking as Dota's does (the items' notes: "Stacks
    // multiplicatively with other sources of magic resistance").
    ItemDef {
        carried: Carried {
            magic_resist_bp: magic_resist(&ITEM_SHAWL, "bonus_magical_armor"),
            ..NOTHING
        },
        ..from(&ITEM_SHAWL)
    },
    ItemDef {
        carried: Carried {
            magic_resist_bp: magic_resist(&ITEM_CLOAK, "bonus_magical_armor"),
            ..NOTHING
        },
        ..from(&ITEM_CLOAK)
    },
    // Morbid Mask: also Liquipedia's *Lifesteal* (7.38: lifesteal heals off physical attack
    // damage alone, and less off what is no hero).
    ItemDef {
        carried: Carried {
            lifesteal_pct: int(&ITEM_LIFESTEAL, "lifesteal_percent"),
            ..NOTHING
        },
        ..from(&ITEM_LIFESTEAL)
    },
    // Voodoo Mask: also Liquipedia's *Spell Lifesteal* and *Voodoo Mask* (several of one source do
    // not stack).
    ItemDef {
        carried: Carried {
            spell_lifesteal_pct: int(&ITEM_VOODOO_MASK, "spell_lifesteal"),
            ..NOTHING
        },
        ..from(&ITEM_VOODOO_MASK)
    },
    // Faerie Fire: eaten at once by its holder alone (no target), for health; its damage while
    // carried.
    ItemDef {
        carried: Carried {
            damage: int(&ITEM_FAERIE_FIRE, "bonus_damage"),
            ..NOTHING
        },
        aim: Some(Aim::Own),
        spends: Spends::One,
        immediate: true,
        effect: UseSpec::Eat {
            health: int(&ITEM_FAERIE_FIRE, "hp_restore"),
            mana: 0,
        },
        ..from(&ITEM_FAERIE_FIRE)
    },
    // Orb of Blight (`item_blight_stone`): Lesser Corruption, the armor its carrier's attacks take
    // off.
    ItemDef {
        on_attack: Some(OnAttack {
            effect: EffectId::CORRUPTION,
            ticks: ticks(&ITEM_BLIGHT_STONE, "corruption_duration"),
            armor: armor_taken(&ITEM_BLIGHT_STONE, "corruption_armor"),
            dps: 0,
            beat_ticks: 0,
            slow_pct: 0,
            ranged_slow_pct: 0,
            restoration_cut_pct: 0,
            // Liquipedia, Orb of Blight: it affects "Allied units", "Buildings", "Ward-type units".
            affects: Affects::Everything,
            pierces_immunity: pierces_immunity(&ITEM_BLIGHT_STONE),
        }),
        ..from(&ITEM_BLIGHT_STONE)
    },
    // Orb of Venom: Poison Attack, at most once each `AbilityCooldown`, decided as the attack is
    // let go (Dota 7.38: "Poison Attack now has a 9s cooldown").
    ItemDef {
        on_attack: Some(OnAttack {
            effect: EffectId::POISON_ATTACK,
            ticks: ticks(&ITEM_ORB_OF_VENOM, "poison_duration"),
            armor: Fixed::ZERO,
            dps: int(&ITEM_ORB_OF_VENOM, "damage"),
            beat_ticks: ticks(&ITEM_ORB_OF_VENOM, "tick_interval"),
            slow_pct: 0,
            ranged_slow_pct: 0,
            restoration_cut_pct: 0,
            // Liquipedia, Orb of Venom: it does not affect "Buildings", "Ward-type units", "Ally
            // units" (Dota 2012-06-11: "Fixed Poison Attack working on denies"; 2011-07-01: "Fixed
            // ability affecting buildings").
            affects: Affects::EnemyUnits,
            pierces_immunity: pierces_immunity(&ITEM_ORB_OF_VENOM),
        }),
        ..from(&ITEM_ORB_OF_VENOM)
    },
    // Orb of Frost: Frost, a slow by the struck's attack and a cut of its Health Restoration (Dota
    // 7.39: "now reduces Health Restoration"; 7.41: "Now also modifies incoming healing").
    ItemDef {
        on_attack: Some(OnAttack {
            effect: EffectId::FROST,
            ticks: ticks(&ITEM_ORB_OF_FROST, "duration"),
            armor: Fixed::ZERO,
            dps: 0,
            beat_ticks: 0,
            slow_pct: -int(&ITEM_ORB_OF_FROST, "slow_melee"),
            ranged_slow_pct: -int(&ITEM_ORB_OF_FROST, "slow_ranged"),
            restoration_cut_pct: int(&ITEM_ORB_OF_FROST, "heal_reduction"),
            // Liquipedia, Orb of Frost: it does not affect "Buildings", "Ward-type units",
            // "Couriers"; Dota 7.41e: "Frost no longer applies when attacking allies".
            affects: Affects::EnemyUnits,
            pierces_immunity: pierces_immunity(&ITEM_ORB_OF_FROST),
        }),
        ..from(&ITEM_ORB_OF_FROST)
    },
    ItemDef {
        limits: ITEM_AEGIS.limits,
        // Valve 2012-03-22: couriers cannot pick up Aegis; this is an engine rule.
        pickup: crate::PickupEligibility::RealHero,
        public_loot: true,
        pickup_lifetime: ticks(&ITEM_AEGIS, "disappear_time"),
        reincarnation: Some(ticks(&ITEM_AEGIS, "reincarnate_time")),
        // The script has no recovery values. Aegis, Reincarnation documents full
        // maximum-pool recovery over five seconds, dispelled by other players' damage.
        expiry_recovery: Some(crate::Recovery {
            ticks: 5 * rules::TICKS_PER_SECOND,
            health_pct: 20,
            mana_pct: 20,
        }),
        ..from(&ITEM_AEGIS)
    },
    ItemDef {
        limits: ITEM_REFRESHER_SHARD.limits,
        cooldown_in_inventory: ITEM_REFRESHER_SHARD.cooldown_in_inventory,
        aim: Some(Aim::Own),
        spends: Spends::One,
        immediate: true,
        effect: UseSpec::Refresh {
            items: int(&ITEM_REFRESHER_SHARD, "refresh_items") != 0,
        },
        ..from(&ITEM_REFRESHER_SHARD)
    },
    ItemDef {
        public_placement: true,
        aim: Some(Aim::Point),
        charges: 1,
        spends: Spends::One,
        effect: UseSpec::Ward {
            units: &crate::BANNER_WARDS,
            ticks: ticks(&ITEM_ROSHANS_BANNER, "duration"),
        },
        limits: ITEM_ROSHANS_BANNER.limits,
        ..from(&ITEM_ROSHANS_BANNER)
    },
    // Infused Raindrops: Magical Damage Block, a charge and its cooldown a block; its charges
    // spent, it is gone. Its mana regeneration while carried.
    ItemDef {
        carried: Carried {
            mana_regen: regen(&ITEM_INFUSED_RAINDROP, "mana_regen"),
            ..NOTHING
        },
        magic_block: Some(MagicBlock {
            amount: int(&ITEM_INFUSED_RAINDROP, "magic_damage_block"),
            min_damage: int(&ITEM_INFUSED_RAINDROP, "min_damage"),
        }),
        ..from(&ITEM_INFUSED_RAINDROP)
    },
    // Ghost Scepter: its attributes, and Ghost Form, ethereal for a while at once and breaking no
    // order (`DOTA_ABILITY_BEHAVIOR_IMMEDIATE`). The ethereal cooldown it shares with Ethereal
    // Blade and Crella's Crozier (`AbilitySharedCooldown`) waits on neither: bota sells neither.
    ItemDef {
        carried: Carried {
            attributes: Attributes::all(int(&ITEM_GHOST, "bonus_all_stats")),
            ..NOTHING
        },
        aim: Some(Aim::Own),
        immediate: true,
        effect: UseSpec::Ethereal {
            ticks: ticks(&ITEM_GHOST, "duration"),
            // "30% more vulnerable to magic damage", written as -30.
            magic_pct: -int(&ITEM_GHOST, "extra_spell_damage_percent"),
        },
        ..from(&ITEM_GHOST)
    },
    // Chasm Stone: its AoE bonus, the best of several carried counting.
    ItemDef {
        carried: Carried {
            aoe_bonus: int(&ITEM_CHASM_STONE, "aoe_bonus"),
            hp: int(&ITEM_CHASM_STONE, "bonus_health"),
            ..NOTHING
        },
        ..from(&ITEM_CHASM_STONE)
    },
    // Upgrades that carry stats and nothing else, built of their parts as the install says.
    ItemDef {
        carried: Carried {
            attributes: points(
                Attribute::Intelligence,
                int(&ITEM_OBLIVION_STAFF, "bonus_intellect"),
            ),
            attack_speed: int(&ITEM_OBLIVION_STAFF, "bonus_attack_speed"),
            mana_regen: regen(&ITEM_OBLIVION_STAFF, "bonus_mana_regen"),
            ..NOTHING
        },
        ..from(&ITEM_OBLIVION_STAFF)
    },
    ItemDef {
        carried: Carried {
            hp_regen: regen(&ITEM_PERS, "bonus_health_regen"),
            mana_regen: regen(&ITEM_PERS, "bonus_mana_regen"),
            ..NOTHING
        },
        ..from(&ITEM_PERS)
    },
    ItemDef {
        carried: Carried {
            hp: int(&ITEM_SOUL_BOOSTER, "bonus_health"),
            mana: int(&ITEM_SOUL_BOOSTER, "bonus_mana"),
            ..NOTHING
        },
        ..from(&ITEM_SOUL_BOOSTER)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_FALCON_BLADE)
    },
    ItemDef {
        carried: Carried {
            hp: int(&ITEM_FALCON_BLADE, "bonus_health"),
            mana_regen: regen(&ITEM_FALCON_BLADE, "bonus_mana_regen"),
            damage: int(&ITEM_FALCON_BLADE, "bonus_damage"),
            ..NOTHING
        },
        ..from(&ITEM_FALCON_BLADE)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_YASHA)
    },
    // Yasha: its movement speed counts once with what is built of it (`lineage`).
    ItemDef {
        carried: Carried {
            attributes: points(Attribute::Agility, int(&ITEM_YASHA, "bonus_agility")),
            attack_speed: int(&ITEM_YASHA, "bonus_attack_speed"),
            move_speed_pct: int(&ITEM_YASHA, "movement_speed_percent_bonus"),
            ..NOTHING
        },
        ..from(&ITEM_YASHA)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_SANGE)
    },
    // Sange: its slow resistance and Health Restoration count once with what is built of it.
    ItemDef {
        carried: Carried {
            attributes: points(Attribute::Strength, int(&ITEM_SANGE, "bonus_strength")),
            slow_resist_bp: bp(&ITEM_SANGE, "slow_resistance"),
            restoration_amp_bp: bp(&ITEM_SANGE, "hp_regen_amp"),
            ..NOTHING
        },
        ..from(&ITEM_SANGE)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_KAYA)
    },
    // Kaya: its spell and mana regeneration amplifications count once with what is built of it.
    ItemDef {
        carried: Carried {
            attributes: points(Attribute::Intelligence, int(&ITEM_KAYA, "bonus_intellect")),
            spell_amp_pct: int(&ITEM_KAYA, "spell_amp"),
            mana_regen_amp_pct: int(&ITEM_KAYA, "mana_regen_multiplier"),
            ..NOTHING
        },
        ..from(&ITEM_KAYA)
    },
    // The three pairs: each holds the stats of the two it is built of, a lineage with each.
    ItemDef {
        carried: sange_and_yasha("movement_speed_percent_bonus_melee"),
        ranged: Some(sange_and_yasha("movement_speed_percent_bonus")),
        ..from(&ITEM_SANGE_AND_YASHA)
    },
    ItemDef {
        carried: Carried {
            attributes: Attributes {
                strength: Fixed::from_int(int(&ITEM_KAYA_AND_SANGE, "bonus_strength")),
                agility: Fixed::ZERO,
                intelligence: Fixed::from_int(int(&ITEM_KAYA_AND_SANGE, "bonus_intellect")),
            },
            slow_resist_bp: bp(&ITEM_KAYA_AND_SANGE, "slow_resistance"),
            restoration_amp_bp: bp(&ITEM_KAYA_AND_SANGE, "hp_regen_amp"),
            spell_amp_pct: int(&ITEM_KAYA_AND_SANGE, "spell_amp"),
            mana_regen_amp_pct: int(&ITEM_KAYA_AND_SANGE, "mana_regen_multiplier"),
            mana_cost_cut_bp: bp(&ITEM_KAYA_AND_SANGE, "manacost_reduction"),
            ..NOTHING
        },
        ..from(&ITEM_KAYA_AND_SANGE)
    },
    ItemDef {
        carried: yasha_and_kaya("movement_speed_percent_bonus_melee"),
        ranged: Some(yasha_and_kaya("movement_speed_percent_bonus")),
        ..from(&ITEM_YASHA_AND_KAYA)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_AETHER_LENS)
    },
    // Aether Lens: every cast of its carrier's reaches further, the best of several counting.
    ItemDef {
        carried: Carried {
            mana: int(&ITEM_AETHER_LENS, "bonus_mana"),
            mana_regen: regen(&ITEM_AETHER_LENS, "bonus_mana_regen"),
            cast_range_bonus: int(&ITEM_AETHER_LENS, "cast_range_bonus"),
            ..NOTHING
        },
        ..from(&ITEM_AETHER_LENS)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_OCTARINE_CORE)
    },
    // Octarine Core: a cut of every cooldown, the best of several counting.
    ItemDef {
        carried: Carried {
            hp: int(&ITEM_OCTARINE_CORE, "bonus_health"),
            mana: int(&ITEM_OCTARINE_CORE, "bonus_mana"),
            hp_regen: regen(&ITEM_OCTARINE_CORE, "bonus_health_regen"),
            mana_regen: regen(&ITEM_OCTARINE_CORE, "bonus_mana_regen"),
            cooldown_cut_bp: bp(&ITEM_OCTARINE_CORE, "bonus_cooldown"),
            ..NOTHING
        },
        ..from(&ITEM_OCTARINE_CORE)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_DRAGON_LANCE)
    },
    // Dragon Lance: attack range for a ranged carrier (`base_attack_range`) and for a melee one
    // (`base_attack_range_melee`), the best of several counting.
    ItemDef {
        carried: Carried {
            attributes: Attributes {
                strength: Fixed::from_int(int(&ITEM_DRAGON_LANCE, "bonus_strength")),
                agility: Fixed::from_int(int(&ITEM_DRAGON_LANCE, "bonus_agility")),
                intelligence: Fixed::ZERO,
            },
            melee_range: int(&ITEM_DRAGON_LANCE, "base_attack_range_melee"),
            ranged_range: int(&ITEM_DRAGON_LANCE, "base_attack_range"),
            ..NOTHING
        },
        ..from(&ITEM_DRAGON_LANCE)
    },
    ItemDef {
        ..from(&ITEM_RECIPE_HEART)
    },
    // Heart of Tarrasque: a share of the maximum health and of the missing health regenerated.
    ItemDef {
        carried: Carried {
            attributes: points(Attribute::Strength, int(&ITEM_HEART, "bonus_strength")),
            max_hp_regen_bp: hundredths_bp(&ITEM_HEART, "hp_regen"),
            missing_hp_regen_bp: hundredths_bp(&ITEM_HEART, "missing_health_regen"),
            ..NOTHING
        },
        ..from(&ITEM_HEART)
    },
];

/// What Sange and Yasha carries, its movement speed of the key a melee or a ranged carrier
/// reads.
const fn sange_and_yasha(move_speed_key: &str) -> Carried {
    let d = &ITEM_SANGE_AND_YASHA;
    Carried {
        attributes: Attributes {
            strength: Fixed::from_int(int(d, "bonus_strength")),
            agility: Fixed::from_int(int(d, "bonus_agility")),
            intelligence: Fixed::ZERO,
        },
        attack_speed: int(d, "bonus_attack_speed"),
        move_speed_pct: int(d, move_speed_key),
        status_resist_bp: bp(d, "status_resistance"),
        slow_resist_bp: bp(d, "slow_resistance"),
        restoration_amp_bp: bp(d, "hp_regen_amp"),
        ..NOTHING
    }
}

/// What Yasha and Kaya carries, its movement speed of the key a melee or a ranged carrier reads.
const fn yasha_and_kaya(move_speed_key: &str) -> Carried {
    let d = &ITEM_YASHA_AND_KAYA;
    Carried {
        attributes: Attributes {
            strength: Fixed::ZERO,
            agility: Fixed::from_int(int(d, "bonus_agility")),
            intelligence: Fixed::from_int(int(d, "bonus_intellect")),
        },
        attack_speed: int(d, "bonus_attack_speed"),
        move_speed_pct: int(d, move_speed_key),
        spell_amp_pct: int(d, "spell_amp"),
        mana_regen_amp_pct: int(d, "mana_regen_multiplier"),
        cast_speed_pct: int(d, "cast_speed_pct"),
        ..NOTHING
    }
}

/// One kind of lotus, as its script gives it.
const fn lotus(d: &ItemData) -> ItemDef {
    ItemDef {
        aim: Some(Aim::Own),
        spends: Spends::One,
        immediate: true,
        ally: Some(AllyUse::Effect { percent: 100 }),
        shared_wait: true,
        take_range: LOTUS_TAKE_RANGE,
        components: &[],
        charge_recipes: d.builds,
        effect: UseSpec::Eat {
            health: int(d, "replenish_amount"),
            mana: int(d, "replenish_amount"),
        },
        ..from(d)
    }
}

// The server beats a grenade's debuff once a second and reads a scroll's band
// as a ring around a building: the script must say the same.
const _: () = {
    assert!(ITEM_BLOOD_GRENADE.first("tick_rate").0 == 1000);
    assert!(ITEM_BLOOD_GRENADE.first("movespeed_slow").0 < 0);
    assert!(ITEM_BLOOD_GRENADE.cast_range > int(&ITEM_BLOOD_GRENADE, "radius"));
    let (near, far) = (
        int(&ITEM_TPSCROLL, "minimum_distance"),
        int(&ITEM_TPSCROLL, "maximum_distance"),
    );
    assert!(near < far);
    assert!(rules::SCROLL_LANDING_RINGS * rules::SCROLL_LANDING_STEP < far);
    assert!(ITEM_TPSCROLL.channel.ticks() < ITEM_TPSCROLL.cooldown.ticks());
};

// What an attack lays is taken off as the item's `SpellDispellableType` says: the effect's row
// agrees with the script.
const _: () = {
    assert!(matches!(ITEM_BLIGHT_STONE.dispel, Dispel::Basic));
    assert!(crate::EFFECTS[EffectId::CORRUPTION.0 as usize].dispellable);
    assert!(ITEM_BLIGHT_STONE.first("corruption_armor").0 < 0);
    assert!(matches!(ITEM_ORB_OF_VENOM.dispel, Dispel::Basic));
    assert!(crate::EFFECTS[EffectId::POISON_ATTACK.0 as usize].dispellable);
    // Frost names no SpellDispellableType; Liquipedia, Orb of Frost: its debuff is "Dispellable by
    // any Dispel sources".
    assert!(matches!(ITEM_ORB_OF_FROST.dispel, Dispel::Unstated));
    assert!(crate::EFFECTS[EffectId::FROST.0 as usize].dispellable);
    assert!(ITEM_ORB_OF_FROST.first("slow_melee").0 < 0);
    assert!(ITEM_ORB_OF_FROST.first("slow_ranged").0 < 0);
    // A beat is a whole number of ticks, and removes a whole share of the damage per second.
    assert!(ticks(&ITEM_ORB_OF_VENOM, "tick_interval") > 0);
    assert!(
        ticks(&ITEM_ORB_OF_VENOM, "tick_interval") as i64 * 1000
            == ITEM_ORB_OF_VENOM.first("tick_interval").0 as i64 * rules::TICKS_PER_SECOND as i64
    );
    assert!(
        (int(&ITEM_ORB_OF_VENOM, "damage") * ticks(&ITEM_ORB_OF_VENOM, "tick_interval") as i32)
            % rules::TICKS_PER_SECOND as i32
            == 0
    );
};

// The radii of the catalogue an AoE bonus widens are the ones the script marks: a value that
// ever stops being marked fails the build here (`affected_by_aoe_increase`).
const _: () = {
    assert!(ITEM_MAGIC_STICK.widened("charge_radius"));
    assert!(ITEM_MAGIC_WAND.widened("charge_radius"));
    assert!(ITEM_BLOOD_GRENADE.widened("radius"));
};

// Ghost Form is taken off by a basic dispel, as `SpellDispellableType` says.
const _: () = {
    assert!(matches!(ITEM_GHOST.dispel, Dispel::Basic));
    assert!(crate::EFFECTS[EffectId::GHOST_FORM.0 as usize].dispellable);
    assert!(ITEM_GHOST.first("extra_spell_damage_percent").0 < 0);
};

// Infused Raindrops is bought with as many charges as its blocks: the script says both.
const _: () =
    assert!(ITEM_INFUSED_RAINDROP.charges as i32 == int(&ITEM_INFUSED_RAINDROP, "initial_charges"));

// A Bottle is bought full: the charges it is bought with are all it holds, and the server fills
// one to them.
const _: () = assert!(ITEM_BOTTLE.charges as i32 == int(&ITEM_BOTTLE, "max_charges"));

const _: () = {
    let mut index = 0;
    while index < ITEM_ROWS.len() {
        if let UseSpec::Ward { units, .. } = ITEM_ROWS[index].effect {
            assert!(!units.is_empty());
        }
        index += 1;
    }
};
