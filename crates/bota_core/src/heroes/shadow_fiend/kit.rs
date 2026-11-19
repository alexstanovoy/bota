//! What Shadow Fiend is made of: his body, his seven abilities, his talent
//! tree and the hero row.

use bota_proto::{AbilityId, Attributes, HeroId};

use super::numbers::{
    FEAST_COOLDOWN, FEAST_MANA, FEAST_RADIUS, PRESENCE_LINGER_TICKS, PRESENCE_RADIUS,
    RAZE_CAST_POINT_MS, RAZE_COOLDOWN, RAZE_DISTANCE, RAZE_MANA, REQUIEM_BACKSWING_MS,
    REQUIEM_CAST_POINT_MS, REQUIEM_COOLDOWN, REQUIEM_LINE_DISTANCE, REQUIEM_MANA,
};
use crate::ability::BARE_ABILITY;
use crate::heroes_data::{NEVERMORE_DARK_LORD, NPC_DOTA_HERO_NEVERMORE as DATA};
use crate::item_data::str_eq;
use crate::unit::NO_GROWTH;
use crate::{
    AbilityDef, Aura, AuraModifier, Growth, HERO, HeroDef, Pierces, Reach, Talent, TalentTree,
    UnitDef, Whom, fixed, per_tick_of, rules, stated_breakable, turn_rate_brads,
};

/// Shadow Fiend: fragile and long-ranged, at his script's numbers. His backswing the script
/// does not hold: Liquipedia's *Shadow Fiend* gives his attack animation as 0.5 + 0.54.
pub const SHADOW_FIEND: UnitDef = UnitDef {
    attributes: Attributes {
        strength: fixed(DATA.strength),
        agility: fixed(DATA.agility),
        intelligence: fixed(DATA.intelligence),
    },
    primary: Some(DATA.primary),
    max_hp: DATA.health,
    max_mana: DATA.mana,
    hp_regen: per_tick_of(DATA.health_regen),
    mana_regen: per_tick_of(DATA.mana_regen),
    damage: DATA.damage_min,
    damage_spread: DATA.damage_max - DATA.damage_min,
    attack_range: DATA.attack_range,
    acquisition: DATA.acquisition,
    attack_time: DATA.attack_rate.0,
    attack_point: DATA.attack_point.0,
    attack_backswing: 540,
    projectile_speed: Some(DATA.projectile_speed),
    armor: DATA.armor.int(),
    magic_resist_pct: DATA.magic_resist.int(),
    move_speed: DATA.move_speed,
    turn_rate: turn_rate_brads(DATA.turn_rate),
    day_vision: DATA.day_vision,
    night_vision: DATA.night_vision,
    per_level: Growth {
        attributes: Attributes {
            strength: fixed(DATA.strength_gain),
            agility: fixed(DATA.agility_gain),
            intelligence: fixed(DATA.intelligence_gain),
        },
        ..NO_GROWTH
    },
    ..HERO
};

const _: () = assert!(!DATA.melee, "Shadow Fiend shoots: a projectile");

/// Shadow Fiend's talents, in his script's order: the left of each tier first.
pub const SHADOW_FIEND_TALENTS: TalentTree = [
    Talent::FeastSoulsPerHero,
    Talent::RazeStackDamage,
    Talent::FeastHealPerSoul,
    Talent::PresenceArmor,
    Talent::RequiemFear,
    Talent::NecroSoulCap,
    Talent::RazeAttackDamage,
    Talent::FeastCastSpeed,
];

const _: () = {
    let mut at = 0;
    while at < SHADOW_FIEND_TALENTS.len() {
        assert!(
            str_eq(SHADOW_FIEND_TALENTS[at].key(), DATA.talents[at]),
            "the script's tree"
        );
        at += 1;
    }
};

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
    id: HeroId::SHADOW_FIEND,
    key: "npc_dota_hero_nevermore",

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
const fn raze(id: AbilityId, key: &'static str, reach: usize) -> AbilityDef {
    AbilityDef {
        id,
        key,

        max_level: rules::ABILITY_MAX_LEVEL,
        passive: false,
        mana: &RAZE_MANA,
        cooldown: &RAZE_COOLDOWN,
        range: RAZE_DISTANCE[reach],
        point: RAZE_CAST_POINT_MS,
        ..BARE_ABILITY
    }
}

pub const RAZE_NEAR: AbilityDef = raze(AbilityId::RAZE_NEAR, "nevermore_shadowraze1", 0);
pub const RAZE_MID: AbilityDef = raze(AbilityId::RAZE_MID, "nevermore_shadowraze2", 1);
pub const RAZE_FAR: AbilityDef = raze(AbilityId::RAZE_FAR, "nevermore_shadowraze3", 2);

pub const REQUIEM: AbilityDef = AbilityDef {
    id: AbilityId::REQUIEM,
    key: "nevermore_requiem",

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
    id: AbilityId::NECROMASTERY,
    key: "nevermore_necromastery",

    max_level: 1,
    ..BARE_ABILITY
};

/// Dota's `nevermore_dark_lord`: a passive aura on the enemies, breakable,
/// that tears their armor by its level, whether or not they see him (the
/// ability asks no sight).
pub const PRESENCE: AbilityDef = AbilityDef {
    id: AbilityId::PRESENCE,
    key: "nevermore_dark_lord",

    max_level: rules::ABILITY_MAX_LEVEL,
    range: PRESENCE_RADIUS,
    pierces: Pierces::Yes,
    aura: Some(Aura {
        growth_clock: None,
        whom: Whom::Enemies,
        reaches: Reach::Living,
        radius: PRESENCE_RADIUS,
        linger: PRESENCE_LINGER_TICKS,
        breakable: stated_breakable(&NEVERMORE_DARK_LORD),
        invulnerable: false,
        water: false,
        modifier: AuraModifier::ArmorBroken,
    }),
    ..BARE_ABILITY
};

pub const FEAST_OF_SOULS: AbilityDef = AbilityDef {
    id: AbilityId::FEAST_OF_SOULS,
    key: "nevermore_frenzy",

    max_level: rules::ABILITY_MAX_LEVEL,
    passive: false,
    immediate: true,
    mana: &FEAST_MANA,
    cooldown: &FEAST_COOLDOWN,
    range: FEAST_RADIUS,
    ..BARE_ABILITY
};
