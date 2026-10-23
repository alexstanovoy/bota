//! What Pudge is made of: his body, his four abilities and the hero row.

use bota_proto::{AbilityId, Aim, Attribute, Attributes, Fixed};

use super::numbers::{
    DISMEMBER_CAST_POINT_MS, DISMEMBER_COOLDOWN, DISMEMBER_MANA, DISMEMBER_RANGE, DISMEMBER_TICKS,
    FLESH_HEAP_RANGE, HOOK_CAST_POINT_MS, HOOK_COOLDOWN, HOOK_MANA, HOOK_RANGE,
    MEAT_SHIELD_COOLDOWN, MEAT_SHIELD_MANA, PUDGE_TURN_RATE_BRADS, ROT_RADIUS,
};
use crate::ability::BARE_ABILITY;
use crate::unit::NO_GROWTH;
use crate::{AbilityDef, Growth, HERO, HeroDef, UnitDef, rules};

/// Pudge: heavy, slow, and swings by hand, at the numbers of the game's
/// `scripts/npc/heroes/npc_dota_hero_pudge.txt` (client 6957): 45 to 51 base
/// attack damage, a base attack time of 1.7 seconds, 2.5 health a second.
pub const PUDGE: UnitDef = UnitDef {
    attributes: Attributes {
        strength: Fixed::from_int(25),
        agility: Fixed::from_int(13),
        intelligence: Fixed::from_int(16),
    },
    primary: Some(Attribute::Strength),
    max_hp: 120,
    max_mana: 75,
    hp_regen: Fixed::from_ratio(5, 2 * rules::TICKS_PER_SECOND as i32),
    damage: 45,
    damage_spread: 6,
    attack_range: 175,
    attack_time: 1700,
    attack_point: 500,
    projectile_speed: None,
    armor: 0,
    move_speed: 280,
    turn_rate: PUDGE_TURN_RATE_BRADS,
    per_level: Growth {
        attributes: Attributes {
            strength: Fixed::from_int(3),
            agility: Fixed::from_ratio(14, 10),
            intelligence: Fixed::from_ratio(18, 10),
        },
        ..NO_GROWTH
    },
    ..HERO
};

/// Pudge's hero row: his body and slots, the flesh heap as his innate and no
/// talents.
pub const HERO_DEF: HeroDef = HeroDef {
    name: "Pudge",
    unit: &PUDGE,
    abilities: &[
        AbilityId::MEAT_HOOK,
        AbilityId::ROT,
        AbilityId::MEAT_SHIELD,
        AbilityId::DISMEMBER,
    ],
    innate: Some(AbilityId::FLESH_HEAP),
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
    point: HOOK_CAST_POINT_MS,
    ..BARE_ABILITY
};

pub const ROT: AbilityDef = AbilityDef {
    name: "Rot",
    max_level: rules::ABILITY_MAX_LEVEL,
    passive: false,
    toggle: true,
    ignores_channel: true,
    mana: &[0, 0, 0, 0],
    cooldown: &[0, 0, 0, 0],
    range: ROT_RADIUS,
    ..BARE_ABILITY
};

/// Pudge's innate: it sits in no slot and is never learned.
pub const FLESH_HEAP: AbilityDef = AbilityDef {
    name: "Flesh Heap",
    max_level: 1,
    range: FLESH_HEAP_RANGE,
    ..BARE_ABILITY
};

pub const MEAT_SHIELD: AbilityDef = AbilityDef {
    name: "Meat Shield",
    max_level: rules::ABILITY_MAX_LEVEL,
    passive: false,
    immediate: true,
    ignores_channel: true,
    mana: &MEAT_SHIELD_MANA,
    cooldown: &MEAT_SHIELD_COOLDOWN,
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
    point: DISMEMBER_CAST_POINT_MS,
    duration: DISMEMBER_TICKS,
    ..BARE_ABILITY
};
