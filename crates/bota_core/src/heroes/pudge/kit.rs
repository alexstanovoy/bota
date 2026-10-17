//! What Pudge is made of: his body, his four abilities and the hero row.

use bota_proto::{AbilityId, Aim, Attribute, Attributes, Fixed};

use super::numbers::{
    DISMEMBER_COOLDOWN, DISMEMBER_MANA, DISMEMBER_RANGE, DISMEMBER_TICKS, FLESH_HEAP_RANGE,
    HOOK_COOLDOWN, HOOK_MANA, HOOK_RANGE, ROT_RADIUS,
};
use crate::ability::BARE_ABILITY;
use crate::unit::NO_GROWTH;
use crate::{AbilityDef, Growth, HERO, HeroDef, UnitDef, rules};

/// Pudge: heavy, slow, and swings by hand.
pub const PUDGE: UnitDef = UnitDef {
    attributes: Attributes {
        strength: Fixed::from_int(25),
        agility: Fixed::from_int(14),
        intelligence: Fixed::from_int(14),
    },
    primary: Some(Attribute::Strength),
    max_hp: 150,
    max_mana: 82,
    damage: 21,
    attack_range: 175,
    attack_time: 1933,
    attack_point: 500,
    projectile_speed: None,
    armor: -1,
    move_speed: 280,
    per_level: Growth {
        attributes: Attributes {
            strength: Fixed::from_ratio(35, 10),
            agility: Fixed::from_ratio(15, 10),
            intelligence: Fixed::from_ratio(16, 10),
        },
        hp: 43,
        mana: 5,
        damage: 1,
        ..NO_GROWTH
    },
    ..HERO
};

/// Pudge's hero row: his body and slots, with no innate ability and no talents.
pub const HERO_DEF: HeroDef = HeroDef {
    name: "Pudge",
    unit: &PUDGE,
    abilities: &[
        AbilityId::MEAT_HOOK,
        AbilityId::ROT,
        AbilityId::FLESH_HEAP,
        AbilityId::DISMEMBER,
    ],
    innate: None,
    talents: None,
};

pub const MEAT_HOOK: AbilityDef = AbilityDef {
    name: "Meat Hook",
    aim: Aim::Point,
    max_level: rules::ABILITY_MAX_LEVEL,
    passive: false,
    mana: &HOOK_MANA,
    cooldown: &HOOK_COOLDOWN,
    range: HOOK_RANGE,
    ..BARE_ABILITY
};

pub const ROT: AbilityDef = AbilityDef {
    name: "Rot",
    max_level: rules::ABILITY_MAX_LEVEL,
    passive: false,
    ignores_channel: true,
    mana: &[0, 0, 0, 0],
    cooldown: &[0, 0, 0, 0],
    range: ROT_RADIUS,
    ..BARE_ABILITY
};

pub const FLESH_HEAP: AbilityDef = AbilityDef {
    name: "Flesh Heap",
    max_level: rules::ABILITY_MAX_LEVEL,
    range: FLESH_HEAP_RANGE,
    ..BARE_ABILITY
};

pub const DISMEMBER: AbilityDef = AbilityDef {
    name: "Dismember",
    aim: Aim::Unit,
    max_level: rules::ULT_MAX_LEVEL,
    passive: false,
    ultimate: true,
    at_an_enemy: true,
    mana: &DISMEMBER_MANA,
    cooldown: &DISMEMBER_COOLDOWN,
    range: DISMEMBER_RANGE,
    duration: DISMEMBER_TICKS,
    ..BARE_ABILITY
};
