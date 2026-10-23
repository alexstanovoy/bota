//! What each `{key}` of a template stands for.

use bota_core::{requiem_fear_ticks, rules};

use super::{Scope, fixed, nums, secs, secs_ms, slash, tenths};

/// The value of a key in a scope, if either knows it.
pub(super) fn value(scope: Scope, key: &str) -> Option<String> {
    let own = match scope {
        Scope::Shared => None,
        Scope::Ability(def) => ability_key(def, key),
        Scope::Item(id) => super::item::key(id, key),
    };
    own.or_else(|| shared(key))
}

/// Keys read from the ability's own entry.
fn ability_key(def: &bota_core::AbilityDef, key: &str) -> Option<String> {
    Some(match key {
        "range" => def.range.to_string(),
        "mana" => nums(def.mana),
        "wait" => slash(def.cooldown, secs),
        "point" => secs_ms(def.point),
        "duration" => secs(def.duration),
        _ => return None,
    })
}

/// The value of a key every template may use.
fn shared(key: &str) -> Option<String> {
    Some(match key {
        "hook_damage" => nums(&rules::HOOK_DAMAGE),
        "rot_dps" => nums(&rules::ROT_DAMAGE_PER_SECOND),
        "rot_slow" => nums(&rules::ROT_SLOW_PCT),
        "heap_strength" => fixed(rules::FLESH_HEAP_STRENGTH_PER_STACK),
        "meat_block" => nums(&rules::MEAT_SHIELD_BLOCK),
        "meat_secs" => slash(&rules::MEAT_SHIELD_TICKS, secs),
        "dismember_damage" => nums(&rules::DISMEMBER_DAMAGE_PER_SECOND),
        "dismember_strength" => slash(&rules::DISMEMBER_STRENGTH_DAMAGE, fixed),
        "dismember_bites" => rules::DISMEMBER_BITES.to_string(),
        "dismember_pull" => rules::DISMEMBER_PULL_SPEED.to_string(),
        "dismember_limit" => rules::DISMEMBER_PULL_LIMIT.to_string(),
        "burst_pct" => rules::COURIER_BURST_PCT.to_string(),
        "burst_secs" => secs(rules::COURIER_BURST_TICKS),
        "shield_secs" => secs(rules::COURIER_SHIELD_TICKS),
        "raze_radius" => rules::RAZE_RADIUS.to_string(),
        "raze_damage" => nums(&rules::RAZE_DAMAGE),
        "raze_soul" => rules::RAZE_DAMAGE_PER_SOUL.to_string(),
        "raze_stack" => nums(&rules::RAZE_STACK_DAMAGE),
        "raze_secs" => secs(rules::RAZE_DEBUFF_TICKS),
        "raze_max_stacks" => rules::RAZE_MAX_STACKS.to_string(),
        "requiem_lines" => rules::REQUIEM_LINES_MAX.to_string(),
        "requiem_damage" => nums(&rules::REQUIEM_LINE_DAMAGE),
        "requiem_fear" => secs(requiem_fear_ticks(false).0),
        "requiem_hold" => secs(requiem_fear_ticks(false).1),
        "requiem_slow" => nums(&rules::REQUIEM_SLOW_PCT),
        "requiem_break" => nums(&rules::REQUIEM_MAGIC_BREAK_PCT),
        "requiem_death_pct" => rules::REQUIEM_DEATH_LINES_PCT.to_string(),
        "souls_hero" => rules::SOULS_PER_HERO.to_string(),
        "soul_damage" => tenths(rules::SOUL_DAMAGE_TENTHS),
        "soul_gain" => tenths(rules::SOUL_DAMAGE_TENTHS_GAIN),
        "soul_levels" => rules::SOUL_DAMAGE_LEVELS.to_string(),
        "soul_cap" => rules::NECRO_SOUL_CAP.to_string(),
        "soul_loss" => rules::SOULS_LOST_ON_DEATH_PCT.to_string(),
        "presence_armor" => slash(&rules::PRESENCE_ARMOR, fixed),
        "feast_secs" => secs(rules::FEAST_TICKS),
        "feast_speed" => nums(&rules::FEAST_ATTACK_SPEED),
        "feast_move" => nums(&rules::FEAST_MOVE_PCT),
        "feast_period" => secs(rules::FEAST_PERIOD_TICKS),
        "feast_per_period" => rules::FEAST_PER_PERIOD.to_string(),
        "feast_hero" => rules::FEAST_SOULS_PER_HERO.to_string(),
        "feast_unit" => rules::FEAST_SOULS_PER_UNIT.to_string(),
        "feast_targets" => slash(&rules::FEAST_TARGETS, |n| n.to_string()),
        "feast_linger" => secs(rules::FEAST_LINGER_TICKS),
        "grenade_radius" => rules::GRENADE_RADIUS.to_string(),
        "grenade_impact" => rules::GRENADE_IMPACT_DAMAGE.to_string(),
        "grenade_dps" => rules::GRENADE_DAMAGE_PER_SECOND.to_string(),
        "grenade_slow" => rules::GRENADE_SLOW_PCT.to_string(),
        "grenade_secs" => secs(rules::GRENADE_DEBUFF_TICKS),
        "magic_range" => rules::MAGIC_CHARGE_RANGE.to_string(),
        "talent_feast_speed" => rules::TALENT_FEAST_ATTACK_SPEED.to_string(),
        "talent_raze_stack" => rules::TALENT_RAZE_STACK_DAMAGE.to_string(),
        "talent_feast_souls" => rules::TALENT_FEAST_SOULS_PER_HERO.to_string(),
        "talent_presence" => fixed(rules::TALENT_PRESENCE_ARMOR),
        "talent_fear" => secs(rules::TALENT_REQUIEM_LINE_TICKS),
        "talent_fear_max" => secs(rules::TALENT_REQUIEM_HOLD_TICKS),
        "talent_souls" => rules::TALENT_NECRO_SOUL_CAP.to_string(),
        "talent_cast_pct" => rules::TALENT_FEAST_CAST_POINT_PCT.to_string(),
        "talent_cast_speed" => (100 - rules::TALENT_FEAST_CAST_POINT_PCT).to_string(),
        _ => return None,
    })
}
