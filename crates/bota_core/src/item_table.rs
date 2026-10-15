//! The catalog itself: one entry to an item, in the order of the ids.

use bota_proto::{Aim, Attribute, Attributes, Fixed, ItemId};

use crate::{
    BLINK_RANGE, Carried, GRENADE_COOLDOWN, GRENADE_RANGE, ItemDef, MANGO_COST, MANGO_HP_REGEN,
    MANGO_MANA, MANGO_STACK_MAX, Mend, NOTHING, OBSERVER_WARD, PLAIN, Pool, Ratio, SALVE_TICKS,
    SENTRY_WARD, Spends, Stock, UseSpec, per_second, points, points_with, rules,
};

/// What Power Treads are built from.
const TREADS_PARTS: [ItemId; 3] = [ItemId::BOOTS, ItemId::GLOVES, ItemId::BELT];
/// What Phase Boots are built from.
const PHASE_PARTS: [ItemId; 4] = [
    ItemId::BOOTS,
    ItemId::BLADES_OF_ATTACK,
    ItemId::BLADES_OF_ATTACK,
    ItemId::RECIPE_PHASE_BOOTS,
];
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
pub const ITEMS: [ItemDef; 54] = [
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
        ..PLAIN
    },
    // Healing Salve.
    ItemDef {
        cost: 110,
        charges: 1,
        aim: Some(Aim::Unit),
        range: 250,
        spends: Spends::One,
        effect: UseSpec::Mend(Mend {
            pool: Pool::Health,
            total: 400,
            ticks: SALVE_TICKS,
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
        aim: Some(Aim::Point),
        range: 350,
        spends: Spends::One,
        effect: UseSpec::PlantTree {
            ticks: rules::PLANTED_TREE_TICKS,
            range: 350,
        },
        ..PLAIN
    },
    // Observer Ward.
    ItemDef {
        cost: 100,
        charges: 1,
        aim: Some(Aim::Point),
        range: 500,
        spends: Spends::One,
        effect: UseSpec::Ward {
            unit: &OBSERVER_WARD,
            ticks: 10800,
            range: 500,
        },
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
        aim: Some(Aim::Tree),
        range: 350,
        effect: UseSpec::FellTree { range: 350 },
        ..PLAIN
    },
    // Sentry Ward.
    ItemDef {
        cost: 50,
        charges: 1,
        aim: Some(Aim::Point),
        range: 500,
        spends: Spends::One,
        effect: UseSpec::Ward {
            unit: &SENTRY_WARD,
            ticks: 12600,
            range: 500,
        },
        ..PLAIN
    },
    // Tango.
    ItemDef {
        cost: 90,
        charges: 3,
        aim: Some(Aim::Tree),
        range: 165,
        spends: Spends::One,
        effect: UseSpec::Mend(Mend {
            pool: Pool::Health,
            total: 115,
            ticks: 480,
            range: 165,
            eats_a_tree: true,
            breaks: false,
        }),
        ..PLAIN
    },
    // Town Portal Scroll: Dota 7.41f `item_tpscroll`. Stacks without limit
    // in Dota; a stack's charges are a byte here, more than a match buys.
    ItemDef {
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
        aim: Some(Aim::Own),
        effect: UseSpec::SwitchMode,
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
        aim: Some(Aim::Own),
        ignores_channel: true,
        effect: UseSpec::Phase { pct: 20, ticks: 93 },
        ..PLAIN
    },
    // Blink Dagger.
    ItemDef {
        cost: 2250,
        cooldown: 450,
        breaks_on_damage: 90,
        aim: Some(Aim::Point),
        range: 1200,
        effect: UseSpec::Blink { range: BLINK_RANGE },
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
        aim: Some(Aim::Own),
        spends: Spends::All,
        effect: UseSpec::Restore {
            hp_per_charge: 15,
            mana_per_charge: 15,
        },
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
        aim: Some(Aim::Own),
        spends: Spends::All,
        effect: UseSpec::Restore {
            hp_per_charge: 15,
            mana_per_charge: 15,
        },
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
    // Enchanted Mango.
    ItemDef {
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
        ..PLAIN
    },
    // Eaglesong.
    ItemDef {
        cost: 2800,
        carried: Carried {
            attributes: points(Attribute::Agility, 25),
            ..NOTHING
        },
        ..PLAIN
    },
    // Claymore.
    ItemDef {
        cost: 1350,
        carried: Carried {
            damage: 20,
            ..NOTHING
        },
        ..PLAIN
    },
    // Talisman of Evasion.
    ItemDef {
        cost: 1300,
        carried: Carried {
            evasion: Ratio::new(3, 20),
            ..NOTHING
        },
        ..PLAIN
    },
    // Butterfly.
    ItemDef {
        cost: 5450,
        components: &BUTTERFLY_PARTS,
        carried: Carried {
            attributes: points(Attribute::Agility, 30),
            damage: 30,
            base_attack_speed_pct: 20,
            evasion: Ratio::new(7, 20),
            ..NOTHING
        },
        ..PLAIN
    },
    // Javelin.
    ItemDef {
        cost: 900,
        carried: Carried {
            pierce: Ratio::new(1, 4),
            pierce_damage: 60,
            ..NOTHING
        },
        ..PLAIN
    },
    // Demon Edge.
    ItemDef {
        cost: 2200,
        carried: Carried {
            damage: 40,
            ..NOTHING
        },
        ..PLAIN
    },
    // Blitz Knuckles.
    ItemDef {
        cost: 1000,
        carried: Carried {
            attack_speed: 35,
            ..NOTHING
        },
        ..PLAIN
    },
    // The recipe a Monkey King Bar is built with.
    ItemDef { cost: 900, ..PLAIN },
    // Monkey King Bar.
    ItemDef {
        cost: 5000,
        components: &MKB_PARTS,
        carried: Carried {
            damage: 50,
            attack_speed: 50,
            melee_range: 50,
            pierce: Ratio::new(4, 5),
            pierce_damage: 70,
            ..NOTHING
        },
        ..PLAIN
    },
    // Wind Lace.
    ItemDef {
        cost: 225,
        carried: Carried {
            lace_speed: 15,
            ..NOTHING
        },
        ..PLAIN
    },
    // Blood Grenade.
    ItemDef {
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
];

const _: () = {
    // A stocked item is bought whole, and is no part of a build.
    let mut at = 0;
    while at < ITEMS.len() {
        let def = &ITEMS[at];
        assert!(
            !def.ignores_channel || (def.point == 0 && def.backswing == 0 && def.duration == 0)
        );
        if let Some(stock) = ITEMS[at].stock {
            assert!(ITEMS[at].components.is_empty());
            assert!(stock.initial <= stock.max);
            assert!(stock.max > 0);
            assert!(stock.restock_ticks > 0);
        }
        let mut part = 0;
        while part < ITEMS[at].components.len() {
            assert!(ITEMS[ITEMS[at].components[part].0 as usize].stock.is_none());
            part += 1;
        }
        at += 1;
    }
};
