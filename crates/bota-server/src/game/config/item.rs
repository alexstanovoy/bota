//! The catalog: what an item costs, what it carries, and what using it does.
//!
//! Every entry answers to an [`ItemId`], which is its place in [`ITEMS`].

use bota_proto::{Aim, Attribute, Attributes, Fixed, ItemId, ItemView, Target};

use crate::game::Inventory;
use crate::game::Ratio;
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
    /// Percent added to the base attack speed and to what agility adds.
    /// Attack speed from items and from what is on the carrier is left
    /// alone.
    pub base_attack_speed_pct: i32,
    /// Share of attacks at the carrier that miss. Of several carried, the
    /// best counts.
    pub evasion: Ratio,
    /// Share of the carrier's attacks that pierce: go through evasion and an
    /// uphill miss, and land bonus magical damage. Of several carried, the
    /// best counts, with its own damage.
    pub pierce: Ratio,
    /// Magical damage a pierce lands alongside the attack.
    pub pierce_damage: i32,
    /// Attack range added to a melee carrier and to nobody else.
    pub melee_range: i32,
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
    base_attack_speed_pct: 0,
    evasion: Ratio::NEVER,
    pierce: Ratio::NEVER,
    pierce_damage: 0,
    melee_range: 0,
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

/// What an item does at a moment of its use, given its user and the slot
/// it sits in. `false`: it did not happen.
pub type UseHook = fn(&mut crate::game::World, crate::game::Entity, usize, Target) -> bool;

/// What an item does when its use ends.
pub type ItemEndHook = fn(&mut crate::game::World, crate::game::Entity, usize, Target);

fn unused(_: &mut crate::game::World, _: crate::game::Entity, _: usize, _: Target) -> bool {
    false
}

fn goes_on(_: &mut crate::game::World, _: crate::game::Entity, _: usize, _: Target) -> bool {
    true
}

fn nothing(_: &mut crate::game::World, _: crate::game::Entity, _: usize, _: Target) {}

fn clarity(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.mend_with(
        user,
        target,
        crate::game::Mend {
            pool: Pool::Mana,
            total: 150,
            ticks: 750,
            range: 250,
            eats_a_tree: false,
            breaks: true,
        },
    )
}

fn salve(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.mend_with(
        user,
        target,
        crate::game::Mend {
            pool: Pool::Health,
            total: 400,
            ticks: SALVE_TICKS,
            range: 250,
            eats_a_tree: false,
            breaks: true,
        },
    )
}

fn branch(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.plant_a_tree(user, target, rules::PLANTED_TREE_TICKS, 350)
}

fn observer(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.stand_ward(user, target, &crate::game::OBSERVER_WARD, 10800, 500)
}

fn quelling(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.fell_a_tree(user, target, 350)
}

fn sentry(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.stand_ward(user, target, &crate::game::SENTRY_WARD, 12600, 500)
}

fn tango(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.mend_with(
        user,
        target,
        crate::game::Mend {
            pool: Pool::Health,
            total: 115,
            ticks: 480,
            range: 165,
            eats_a_tree: true,
            breaks: false,
        },
    )
}

fn scroll_reads(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.may_teleport(user, target, 600)
}

fn scroll_carries(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    slot: usize,
    target: Target,
) {
    world.teleport_to(user, slot, target);
}

fn treads(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    slot: usize,
    _: Target,
) -> bool {
    world.switch_mode(user, slot)
}

fn phase(world: &mut crate::game::World, user: crate::game::Entity, _: usize, _: Target) -> bool {
    world.walk_through(user, 20, 93)
}

fn blink(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.blink_to(user, target, BLINK_RANGE)
}

fn stick(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    slot: usize,
    _: Target,
) -> bool {
    let charges = i32::from(world.charges_in(user, slot));
    world.restore_with(user, charges * 15, charges * 15)
}

fn mango(
    world: &mut crate::game::World,
    user: crate::game::Entity,
    _: usize,
    target: Target,
) -> bool {
    world.replenish_mana(user, target, MANGO_MANA)
}

/// One entry of the catalog.
#[derive(Clone, Copy, Debug)]
pub struct ItemDef {
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
    /// What it adds to whoever carries it; health regeneration is per charge when mergeable.
    pub carried: Carried,
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
    /// At the moment of use. `false`: nothing happened and nothing is spent.
    pub on_use: UseHook,
    /// Every tick it runs on. `false`: it breaks off.
    pub on_during: UseHook,
    /// Broken off by an order, a stun, the user falling, or its own `false`.
    pub on_cancel: ItemEndHook,
    /// Run to the end of `duration`.
    pub on_complete: ItemEndHook,
}

/// An item that costs nothing, carries nothing and does nothing, so an entry
/// names only what sets it apart.
const PLAIN: ItemDef = ItemDef {
    cost: 0,
    charges: 0,
    stack_limit: 0,
    cast_charges: 0,
    cooldown: 0,
    shared_wait: false,
    mana_cost: 0,
    breaks_on_damage: 0,
    mode: None,
    mode_bonus: 0,
    components: &[],
    carried: NOTHING,
    aim: None,
    range: 0,
    spends: Spends::Nothing,
    point: 0,
    backswing: 0,
    duration: 0,
    mana_deficit: false,
    on_use: unused,
    on_during: goes_on,
    on_cancel: nothing,
    on_complete: nothing,
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
/// Enchanted Mango.
pub const ITEM_MANGO: u16 = 42;
/// Eaglesong.
pub const ITEM_EAGLESONG: u16 = 43;
/// Claymore.
pub const ITEM_CLAYMORE: u16 = 44;
/// Talisman of Evasion.
pub const ITEM_TALISMAN_OF_EVASION: u16 = 45;
/// Butterfly.
pub const ITEM_BUTTERFLY: u16 = 46;
/// Javelin.
pub const ITEM_JAVELIN: u16 = 47;
/// Demon Edge.
pub const ITEM_DEMON_EDGE: u16 = 48;
/// Blitz Knuckles.
pub const ITEM_BLITZ_KNUCKLES: u16 = 49;
/// The recipe a Monkey King Bar is built with.
pub const ITEM_RECIPE_MONKEY_KING_BAR: u16 = 50;
/// Monkey King Bar.
pub const ITEM_MONKEY_KING_BAR: u16 = 51;

/// Gold per Mango charge, bought one at a time.
pub const MANGO_COST: i32 = 65;
/// Maximum Mango charges in one slot.
pub const MANGO_STACK_MAX: u8 = 3;
/// Mana restored by one Mango charge, capped at the user's maximum.
pub const MANGO_MANA: i32 = 100;
/// Ticks a Healing Salve mends over.
pub const SALVE_TICKS: u32 = 300;
/// How far a Blink Dagger carries, in units.
pub const BLINK_RANGE: i32 = 1200;
/// Health per tick per charge: `2 / (5 * TICKS_PER_SECOND)`, truncated to Q16.16.
/// At 30 ticks/s this is 873 raw/tick, exactly 0.399627685546875 HP/s.
pub const MANGO_HP_REGEN: Fixed = Fixed::from_ratio(2, 5 * rules::TICKS_PER_SECOND as i32);

const _: () = {
    assert!(MANGO_COST > 0);
    assert!(MANGO_STACK_MAX > 1);
    assert!(MANGO_MANA > 0);
    assert!(MANGO_HP_REGEN.raw > 0);
};

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
/// What a Butterfly is built from.
const BUTTERFLY_PARTS: [ItemId; 3] = [
    ItemId(ITEM_EAGLESONG),
    ItemId(ITEM_CLAYMORE),
    ItemId(ITEM_TALISMAN_OF_EVASION),
];
/// What a Monkey King Bar is built from.
const MKB_PARTS: [ItemId; 4] = [
    ItemId(ITEM_DEMON_EDGE),
    ItemId(ITEM_BLITZ_KNUCKLES),
    ItemId(ITEM_JAVELIN),
    ItemId(ITEM_RECIPE_MONKEY_KING_BAR),
];

/// The catalog, indexed by [`ItemId`].
pub const ITEMS: [ItemDef; 52] = [
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
        on_use: clarity,
        ..PLAIN
    },
    // Healing Salve.
    ItemDef {
        cost: 110,
        charges: 1,
        aim: Some(Aim::Unit),
        range: 250,
        spends: Spends::One,
        on_use: salve,
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
        on_use: branch,
        ..PLAIN
    },
    // Observer Ward.
    ItemDef {
        cost: 100,
        charges: 1,
        aim: Some(Aim::Point),
        range: 500,
        spends: Spends::One,
        on_use: observer,
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
        on_use: quelling,
        ..PLAIN
    },
    // Sentry Ward.
    ItemDef {
        cost: 50,
        charges: 1,
        aim: Some(Aim::Point),
        range: 500,
        spends: Spends::One,
        on_use: sentry,
        ..PLAIN
    },
    // Tango.
    ItemDef {
        cost: 90,
        charges: 3,
        aim: Some(Aim::Tree),
        range: 165,
        spends: Spends::One,
        on_use: tango,
        ..PLAIN
    },
    // Town Portal Scroll.
    ItemDef {
        cost: 100,
        charges: 1,
        cooldown: rules::SCROLL_WAIT_TICKS,
        shared_wait: true,
        aim: Some(Aim::Building),
        range: 600,
        duration: 90,
        on_use: scroll_reads,
        on_complete: scroll_carries,
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
        on_use: treads,
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
        on_use: phase,
        ..PLAIN
    },
    // Blink Dagger.
    ItemDef {
        cost: 2250,
        cooldown: 450,
        breaks_on_damage: 90,
        aim: Some(Aim::Point),
        range: 1200,
        on_use: blink,
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
        on_use: stick,
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
        on_use: stick,
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
        on_use: mango,
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
];

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
                    range: def.map_or(0, |def| def.range),
                    aim: def.and_then(|def| def.aim),
                    for_sale: stack.for_sale,
                }
            })
        })
        .collect()
}
