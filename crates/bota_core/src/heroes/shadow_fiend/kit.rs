//! What Shadow Fiend is made of: his body, his seven abilities, his talent
//! tree and the hero row.

use bota_proto::{AbilityId, Attribute, Attributes, Fixed};

use super::numbers::{
    FEAST_COOLDOWN, FEAST_MANA, FEAST_RADIUS, PRESENCE_LINGER_TICKS, PRESENCE_RADIUS,
    RAZE_CAST_POINT_MS, RAZE_COOLDOWN, RAZE_DISTANCE, RAZE_MANA, REQUIEM_BACKSWING_MS,
    REQUIEM_CAST_POINT_MS, REQUIEM_COOLDOWN, REQUIEM_LINE_DISTANCE, REQUIEM_MANA,
};
use crate::ability::BARE_ABILITY;
use crate::unit::NO_GROWTH;
use crate::{
    AbilityDef, Aura, AuraModifier, Growth, HERO, HeroDef, Pierces, Reach, Talent, TalentTree,
    UnitDef, Whom, rules,
};

/// Shadow Fiend: fragile and long-ranged, at Dota 7.41f's numbers, 16 to 22
/// base attack damage among them.
pub const SHADOW_FIEND: UnitDef = UnitDef {
    attributes: Attributes {
        strength: Fixed::from_int(19),
        agility: Fixed::from_int(25),
        intelligence: Fixed::from_int(16),
    },
    primary: Some(Attribute::Agility),
    max_hp: 120,
    max_mana: 75,
    mana_regen: Fixed::from_ratio(3, 10 * rules::TICKS_PER_SECOND as i32),
    damage: 16,
    damage_spread: 6,
    attack_range: 525,
    acquisition: 800,
    attack_time: 1600,
    attack_point: 500,
    attack_backswing: 540,
    projectile_speed: Some(1200),
    armor: 0,
    move_speed: 305,
    turn_rate: rules::FAST_TURN_RATE_BRADS,
    per_level: Growth {
        attributes: Attributes {
            strength: Fixed::from_ratio(27, 10),
            agility: Fixed::from_ratio(36, 10),
            intelligence: Fixed::from_ratio(22, 10),
        },
        ..NO_GROWTH
    },
    ..HERO
};

/// Shadow Fiend's tree at Dota 7.41f, the left of each tier first.
pub const SHADOW_FIEND_TALENTS: TalentTree = [
    Talent::FeastAttackSpeed,
    Talent::RazeStackDamage,
    Talent::FeastSoulsPerHero,
    Talent::PresenceArmor,
    Talent::RequiemFear,
    Talent::NecroSoulCap,
    Talent::RazeAttackDamage,
    Talent::FeastCastSpeed,
];

/// The three razes by reach, nearest first, in the order of
/// [`RAZE_DISTANCE`] they land by.
pub const SHADOW_FIEND_RAZES: [AbilityId; 3] = [
    AbilityId::RAZE_NEAR,
    AbilityId::RAZE_MID,
    AbilityId::RAZE_FAR,
];

const _: () = assert!(SHADOW_FIEND_RAZES.len() == RAZE_DISTANCE.len());

/// Shadow Fiend's hero row: the one hero at Dota's numbers, with a talent
/// tree and the necromastery as his innate.
pub const HERO_DEF: HeroDef = HeroDef {
    name: "Shadow Fiend",
    unit: &SHADOW_FIEND,
    abilities: &[
        SHADOW_FIEND_RAZES[0],
        SHADOW_FIEND_RAZES[1],
        SHADOW_FIEND_RAZES[2],
        AbilityId::FEAST_OF_SOULS,
        AbilityId::PRESENCE,
        AbilityId::REQUIEM,
    ],
    innate: Some(AbilityId::NECROMASTERY),
    talents: Some(&SHADOW_FIEND_TALENTS),
};

/// The three razes are one skill worn as three slots, each landing at its
/// own reach.
const fn raze(name: &'static str, reach: usize) -> AbilityDef {
    AbilityDef {
        name,
        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &RAZE_MANA,
        cooldown: &RAZE_COOLDOWN,
        range: RAZE_DISTANCE[reach],
        point: RAZE_CAST_POINT_MS,
        ..BARE_ABILITY
    }
}

pub const RAZE_NEAR: AbilityDef = raze("Shadowraze (Near)", 0);
pub const RAZE_MID: AbilityDef = raze("Shadowraze (Mid)", 1);
pub const RAZE_FAR: AbilityDef = raze("Shadowraze (Far)", 2);

pub const REQUIEM: AbilityDef = AbilityDef {
    name: "Requiem of Souls",
    max_level: rules::ULT_MAX_LEVEL,
    passive: false,
    ultimate: true,
    mana: &REQUIEM_MANA,
    cooldown: &REQUIEM_COOLDOWN,
    range: REQUIEM_LINE_DISTANCE,
    point: REQUIEM_CAST_POINT_MS,
    backswing: REQUIEM_BACKSWING_MS,
    ..BARE_ABILITY
};

pub const NECROMASTERY: AbilityDef = AbilityDef {
    name: "Necromastery",
    max_level: 1,
    ..BARE_ABILITY
};

/// Dota's `nevermore_dark_lord`: a passive aura on the enemies, breakable,
/// that tears their armor by its level. Only an enemy whose side sees him
/// wears it.
pub const PRESENCE: AbilityDef = AbilityDef {
    name: "Presence of the Dark Lord",
    max_level: rules::ABILITY_MAX_LEVEL,
    range: PRESENCE_RADIUS,
    pierces: Pierces::Yes,
    aura: Some(Aura {
        whom: Whom::Enemies,
        reaches: Reach::Living,
        radius: PRESENCE_RADIUS,
        linger: PRESENCE_LINGER_TICKS,
        breakable: true,
        invulnerable: false,
        seen_only: true,
        water: false,
        modifier: AuraModifier::ArmorBroken,
    }),
    ..BARE_ABILITY
};

pub const FEAST_OF_SOULS: AbilityDef = AbilityDef {
    name: "Feast of Souls",
    max_level: rules::ABILITY_MAX_LEVEL,
    passive: false,
    immediate: true,
    mana: &FEAST_MANA,
    cooldown: &FEAST_COOLDOWN,
    range: FEAST_RADIUS,
    ..BARE_ABILITY
};
