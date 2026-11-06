//! What Pudge is made of: his body, his four abilities and the hero row.

use bota_proto::{AbilityId, Aim, Attributes, HeroId};

use super::numbers::{
    DISMEMBER_CAST_POINT_MS, DISMEMBER_COOLDOWN, DISMEMBER_MANA, DISMEMBER_RANGE, DISMEMBER_TICKS,
    FLESH_HEAP_RANGE, HOOK_CAST_POINT_MS, HOOK_COOLDOWN, HOOK_MANA, HOOK_RANGE,
    MEAT_SHIELD_COOLDOWN, MEAT_SHIELD_MANA, PUDGE_TURN_RATE_BRADS, ROT_RADIUS,
};
use crate::ability::BARE_ABILITY;
use crate::heroes_data::{
    NPC_DOTA_HERO_PUDGE as DATA, PUDGE_DISMEMBER, PUDGE_FLESH_HEAP, PUDGE_INNATE_GRAFT_FLESH,
    PUDGE_MEAT_HOOK, PUDGE_ROT,
};
use crate::item_data::str_eq;
use crate::unit::NO_GROWTH;
use crate::{
    AbilityData, AbilityDef, Behavior, Growth, HERO, HeroDef, Pierces, TargetFilter, TargetTeam,
    UnitDef, UnitTargets, fixed, per_tick_of,
};

/// Pudge: heavy, slow, and swings by hand, at his script's numbers. His backswing the script
/// does not hold: Liquipedia's *Pudge* gives his attack animation as 0.5 + 1.17.
pub const PUDGE: UnitDef = UnitDef {
    attributes: Attributes {
        strength: fixed(DATA.strength),
        agility: fixed(DATA.agility),
        intelligence: fixed(DATA.intelligence),
    },
    primary: Some(DATA.primary),
    max_hp: DATA.health,
    max_mana: DATA.mana,
    hp_regen: per_tick_of(DATA.health_regen),
    damage: DATA.damage_min,
    damage_spread: DATA.damage_max - DATA.damage_min,
    attack_range: DATA.attack_range,
    acquisition: DATA.acquisition,
    attack_time: DATA.attack_rate.0,
    attack_point: DATA.attack_point.0,
    attack_backswing: 1170,
    projectile_speed: None,
    armor: DATA.armor.int(),
    magic_resist_pct: DATA.magic_resist.int(),
    move_speed: DATA.move_speed,
    turn_rate: PUDGE_TURN_RATE_BRADS,
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

const _: () = assert!(DATA.melee, "Pudge swings by hand: no projectile");

/// Pudge's hero row: his body and slots, the flesh heap as his innate and no
/// talents.
pub const HERO_DEF: HeroDef = HeroDef {
    id: HeroId::PUDGE,
    key: "npc_dota_hero_pudge",
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
    id: AbilityId::MEAT_HOOK,
    key: PUDGE_MEAT_HOOK.key,
    name: "Meat Hook",
    aim: Aim::Point,
    max_level: PUDGE_MEAT_HOOK.max_level,
    passive: false,
    mana: &HOOK_MANA,
    cooldown: &HOOK_COOLDOWN,
    range: HOOK_RANGE,
    point: HOOK_CAST_POINT_MS,
    // It pierces spell immunity, but a courier it does not catch.
    pierces: Pierces::NotCouriers,
    ..BARE_ABILITY
};

pub const ROT: AbilityDef = AbilityDef {
    id: AbilityId::ROT,
    key: PUDGE_ROT.key,
    name: "Rot",
    max_level: PUDGE_ROT.max_level,
    passive: false,
    toggle: PUDGE_ROT.behavior.has(Behavior::TOGGLE),
    ignores_channel: PUDGE_ROT.behavior.has(Behavior::IGNORE_CHANNEL),
    mana: &ROT_MANA,
    cooldown: &ROT_COOLDOWN,
    range: ROT_RADIUS,
    ..BARE_ABILITY
};

/// Pudge's innate: it sits in no slot and is never learned.
pub const FLESH_HEAP: AbilityDef = AbilityDef {
    id: AbilityId::FLESH_HEAP,
    key: PUDGE_INNATE_GRAFT_FLESH.key,
    name: "Flesh Heap",
    max_level: PUDGE_INNATE_GRAFT_FLESH.max_level,
    range: FLESH_HEAP_RANGE,
    ..BARE_ABILITY
};

pub const MEAT_SHIELD: AbilityDef = AbilityDef {
    id: AbilityId::MEAT_SHIELD,
    key: PUDGE_FLESH_HEAP.key,
    name: "Meat Shield",
    max_level: PUDGE_FLESH_HEAP.max_level,
    passive: false,
    immediate: PUDGE_FLESH_HEAP.behavior.has(Behavior::IMMEDIATE),
    ignores_channel: PUDGE_FLESH_HEAP.behavior.has(Behavior::IGNORE_CHANNEL),
    mana: &MEAT_SHIELD_MANA,
    cooldown: &MEAT_SHIELD_COOLDOWN,
    ..BARE_ABILITY
};

pub const DISMEMBER: AbilityDef = AbilityDef {
    id: AbilityId::DISMEMBER,
    key: PUDGE_DISMEMBER.key,
    name: "Dismember",
    block: crate::BlockTiming::Cast,
    aim: Aim::Unit,
    max_level: PUDGE_DISMEMBER.max_level,
    passive: false,
    ultimate: PUDGE_DISMEMBER.ultimate,
    at_an_enemy: true,
    mana: &DISMEMBER_MANA,
    cooldown: &DISMEMBER_COOLDOWN,
    range: DISMEMBER_RANGE,
    point: DISMEMBER_CAST_POINT_MS,
    duration: DISMEMBER_TICKS,
    resisted_channel: true,
    // It holds a courier too, whose hits its bites take nothing of.
    pierces: Pierces::Yes,
    targets: TargetFilter::ENEMY_HEROES_AND_BASIC,
    ..BARE_ABILITY
};

/// Rot's cost and wait, by level: the script gives it none.
const ROT_MANA: [i32; 4] = crate::ints_of(PUDGE_ROT.mana);
const ROT_COOLDOWN: [u32; 4] = crate::ticks_of(PUDGE_ROT.cooldown);

/// What bota writes of each ability beside its script, checked against the script: its aim, its
/// reach of a debuff-immune enemy, and whom it is aimed at.
const _: () = {
    let aimed = [
        (&PUDGE_MEAT_HOOK, Behavior::POINT),
        (&PUDGE_ROT, Behavior::NO_TARGET),
        (&PUDGE_FLESH_HEAP, Behavior::NO_TARGET),
        (&PUDGE_INNATE_GRAFT_FLESH, Behavior::PASSIVE),
        (&PUDGE_DISMEMBER, Behavior::UNIT_TARGET),
    ];
    let mut at = 0;
    while at < aimed.len() {
        let (data, aim): (&AbilityData, Behavior) = aimed[at];
        assert!(data.behavior.has(aim));
        assert!(
            str_eq(DATA.abilities[at], data.key),
            "slot order as the script's"
        );
        at += 1;
    }
    assert!(DATA.abilities.len() == aimed.len());
    assert!(matches!(MEAT_HOOK.pierces, Pierces::NotCouriers));
    assert!(matches!(PUDGE_MEAT_HOOK.pierces_immunity, Some(true)));
    assert!(matches!(PUDGE_ROT.pierces_immunity, Some(false)));
    assert!(matches!(DISMEMBER.pierces, Pierces::Yes));
    assert!(matches!(PUDGE_DISMEMBER.pierces_immunity, Some(true)));
    assert!(matches!(
        PUDGE_DISMEMBER.target_team,
        Some(TargetTeam::Enemy)
    ));
    assert!(
        PUDGE_DISMEMBER
            .target_types
            .is(UnitTargets::HERO.and(UnitTargets::BASIC))
    );
    assert!(PUDGE_INNATE_GRAFT_FLESH.innate);
    assert!(!PUDGE_DISMEMBER.behavior.has(Behavior::IMMEDIATE));
};
