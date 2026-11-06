//! Read-only ability projections from catalogue metadata and the mechanics' own values.
#![allow(
    clippy::float_arithmetic,
    reason = "finite observation projections do not change simulation state"
)]

use bota_core::{Act, Buff, Dec, NeutralAbility, Spell, Stat, Strike, Throe, at_level, rules};
use bota_proto::{AbilityId, DamageKind, EffectId, UnitKind, UnitView};

/// How an ability contributes damage to an observation-based estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageCadence {
    Cast,
    Continuous,
    Attack,
}

#[derive(Clone, Copy, Debug)]
pub struct AbilityDamage {
    pub amount: f64,
    pub kind: DamageKind,
    pub cadence: DamageCadence,
}

/// Public observations needed by an ability's damage model.
#[derive(Clone, Copy, Debug)]
pub struct AbilityObservation<'a> {
    pub actor: &'a UnitView,
    pub target: &'a UnitView,
    pub clock: u32,
    pub schedule: rules::Schedule,
}

fn hit(amount: f64, kind: DamageKind) -> Option<AbilityDamage> {
    Some(AbilityDamage {
        amount,
        kind,
        cadence: DamageCadence::Cast,
    })
}
fn continuous(amount: f64) -> Option<AbilityDamage> {
    Some(AbilityDamage {
        amount,
        kind: DamageKind::Magical,
        cadence: DamageCadence::Continuous,
    })
}

/// Damage of one application during `seconds`, before mitigation. The supplied views and
/// clock are observations; no world or hidden state is read. Non-damaging abilities return None.
pub fn ability_damage_projection(
    id: AbilityId,
    level: u8,
    seconds: f64,
    observation: AbilityObservation<'_>,
) -> Option<AbilityDamage> {
    let AbilityObservation {
        actor,
        target: _,
        clock: _,
        schedule: _,
    } = observation;
    if level == 0 || !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    bota_core::ability_def(id)?;
    if let Some(ability) = NeutralAbility::of(id) {
        return neutral_damage(ability.spell(), level, seconds, observation);
    }
    let rank = usize::from(level - 1);
    let souls = actor
        .effects
        .iter()
        .find(|effect| effect.id == EffectId::SOULS)
        .and_then(|effect| effect.stacks)
        .unwrap_or(0);
    match id {
        AbilityId::RAZE_NEAR | AbilityId::RAZE_MID | AbilityId::RAZE_FAR => hit(
            f64::from(bota_core::heroes::shadow_fiend::raze_damage(
                rank.min(rules::RAZE_DAMAGE.len() - 1),
                souls,
            )),
            DamageKind::Magical,
        ),
        AbilityId::REQUIEM => hit(
            f64::from(bota_core::heroes::shadow_fiend::requiem_line_damage(rank))
                * f64::from(bota_core::heroes::shadow_fiend::requiem_lines(souls)),
            DamageKind::Magical,
        ),
        AbilityId::MEAT_HOOK => hit(
            f64::from(pick(&rules::HOOK_DAMAGE, level)),
            DamageKind::Pure,
        ),
        AbilityId::ROT => {
            continuous(f64::from(pick(&rules::ROT_DAMAGE_PER_SECOND, level)) * seconds)
        }
        AbilityId::DISMEMBER => {
            let at = rank.min(rules::DISMEMBER_DAMAGE_PER_SECOND.len() - 1);
            let dismember =
                bota_core::ability_data("pudge_dismember").expect("Pudge's dismember is imported");
            let channel = dismember.value("AbilityChannelTime").at(at).secs();
            let bites = u32::try_from(dismember.value("ticks").at(at).int())
                .expect("a dismember bites a handful of times");
            let landed = (0..bites)
                .filter(|&bite| {
                    f64::from(bota_core::heroes::pudge::dismember_bite_tick(
                        bite, bites, channel,
                    )) < seconds * f64::from(rules::TICKS_PER_SECOND)
                })
                .count();
            let bite = bota_core::heroes::pudge::dismember_bite_damage(
                dismember.value("dismember_damage").at(at),
                dismember.value("strength_damage").at(at),
                actor.attributes.strength,
                channel,
                bites,
            );
            hit(f64::from(bite) * landed as f64, DamageKind::Magical)
        }
        _ => None,
    }
}

fn neutral_damage(
    spell: &Spell,
    level: u8,
    seconds: f64,
    observation: AbilityObservation<'_>,
) -> Option<AbilityDamage> {
    let AbilityObservation {
        actor,
        target,
        clock,
        schedule,
    } = observation;
    match *spell {
        Spell::Cast { act, .. } => cast_damage(act, level, target, seconds, clock, schedule),
        Spell::Storm {
            radius,
            near_radius,
            near_dps,
            far_dps,
            ..
        } => {
            let distance = (bota_core::isqrt64(actor.pos.distance_squared(target.pos)) / 65536)
                .min(i64::from(i32::MAX)) as i32;
            let dps = if distance <= near_radius {
                near_dps
            } else {
                let span = i64::from(radius - near_radius).max(1);
                (i64::from(far_dps)
                    + i64::from(near_dps - far_dps) * i64::from((radius - distance).max(0)) / span)
                    as i32
            };
            continuous(f64::from(dps) * seconds)
        }
        Spell::Strike(Strike::Bash {
            chance_pct, damage, ..
        }) => Some(AbilityDamage {
            amount: f64::from(at_level(&damage, level)) * f64::from(chance_pct) / 100.0,
            kind: DamageKind::Physical,
            cadence: DamageCadence::Attack,
        }),
        Spell::Strike(Strike::Crit {
            chance_pct,
            mult_pct,
        }) => Some(AbilityDamage {
            amount: f64::from(actor.attack_damage.max(0))
                * f64::from((at_level(&mult_pct, level) - 100).max(0))
                * f64::from(chance_pct)
                / 10000.0,
            kind: DamageKind::Physical,
            cadence: DamageCadence::Attack,
        }),
        Spell::Strike(Strike::Lay {
            dps,
            hero_ms,
            other_ms,
            ..
        }) => continuous(
            f64::from(at_level(&dps, level))
                * seconds.min(
                    f64::from(if target.kind == UnitKind::Hero {
                        hero_ms
                    } else {
                        other_ms
                    }) / 1000.0,
                ),
        ),
        Spell::Strike(Strike::Petrify {
            below_pct, ms, dps, ..
        }) if i64::from(actor.hp) * 100 < i64::from(actor.max_hp) * i64::from(below_pct) => {
            Some(AbilityDamage {
                amount: f64::from(dps) * seconds.min(f64::from(at_level(&ms, level)) / 1000.0),
                kind: DamageKind::Physical,
                cadence: DamageCadence::Continuous,
            })
        }
        Spell::Uncast
        | Spell::Idle
        | Spell::Upgrade
        | Spell::Piercing
        | Spell::Aura(_)
        | Spell::Intrinsic { .. }
        | Spell::Block { .. }
        | Spell::Throe(_)
        | Spell::Strike(_) => None,
    }
}

fn cast_damage(
    act: Act,
    level: u8,
    target: &UnitView,
    seconds: f64,
    clock: u32,
    schedule: rules::Schedule,
) -> Option<AbilityDamage> {
    match act {
        Act::Stomp {
            damage,
            growth,
            other_pct,
            kind,
            ..
        } => {
            let base = at_level(&damage, level);
            let amount = growth.map_or(base, |growth| growth.amount(base, level, clock, schedule));
            let scale = if target.kind == UnitKind::Hero {
                100
            } else {
                other_pct
            };
            hit(f64::from(amount) * f64::from(scale) / 100.0, kind)
        }
        Act::Bolt {
            hero_damage,
            other_damage,
            ..
        } => hit(
            f64::from(if target.kind == UnitKind::Hero {
                hero_damage
            } else {
                other_damage
            }),
            DamageKind::Magical,
        ),
        Act::Chain { damage, .. } | Act::Spikes { damage, .. } => {
            hit(f64::from(at_level(&damage, level)), DamageKind::Magical)
        }
        Act::Smash {
            damage,
            current_pct,
            ..
        } => hit(
            f64::from(at_level(&damage, level))
                + f64::from(target.hp.max(0)) * f64::from(current_pct) / 100.0,
            DamageKind::Magical,
        ),
        Act::Roar(spec) => {
            if seconds * f64::from(rules::TICKS_PER_SECOND) <= f64::from(spec.windup) {
                None
            } else {
                hit(
                    f64::from(spec.growth.amount(spec.damage, level, clock, schedule)),
                    DamageKind::Magical,
                )
            }
        }
        Act::Grab(spec) => {
            if target.kind != UnitKind::Hero
                || target
                    .effects
                    .iter()
                    .any(|effect| effect.id == EffectId::ILLUSION)
                || seconds * f64::from(rules::TICKS_PER_SECOND)
                    < f64::from(spec.duration.saturating_add(spec.flight))
            {
                return None;
            }
            hit(
                f64::from(spec.damage)
                    + f64::from(target.hp.max(0))
                        * f64::from(spec.health_growth.amount(0, level, clock, schedule))
                        / 10000.0,
                DamageKind::Pure,
            )
        }
        Act::Summon { .. }
        | Act::Enrage { .. }
        | Act::Bubble { .. }
        | Act::Push { .. }
        | Act::Purge { .. }
        | Act::Tornado { .. } => None,
    }
}

/// The base damage reported by an ability's catalogue, without caster-dependent bonuses.
pub fn ability_base_damage(id: AbilityId, level: u8) -> i32 {
    if bota_core::ability_def(id).is_none() {
        return 0;
    }
    if let Some(ability) = NeutralAbility::of(id) {
        return match *ability.spell() {
            Spell::Cast {
                act:
                    Act::Stomp { damage, .. }
                    | Act::Chain { damage, .. }
                    | Act::Smash { damage, .. }
                    | Act::Spikes { damage, .. },
                ..
            } => at_level(&damage, level),
            Spell::Cast {
                act: Act::Bolt { hero_damage, .. },
                ..
            } => hero_damage,
            Spell::Cast {
                act: Act::Roar(spec),
                ..
            } => spec.damage,
            Spell::Cast {
                act: Act::Grab(spec),
                ..
            } => spec.damage,
            Spell::Storm { near_dps, .. } => near_dps,
            Spell::Strike(Strike::Bash { damage, .. }) => at_level(&damage, level),
            Spell::Strike(Strike::Lay { dps, .. }) => at_level(&dps, level),
            Spell::Strike(Strike::Petrify { dps, .. }) => dps,
            _ => 0,
        };
    }
    match id {
        AbilityId::RAZE_NEAR | AbilityId::RAZE_MID | AbilityId::RAZE_FAR => {
            pick(&rules::RAZE_DAMAGE, level)
        }
        AbilityId::REQUIEM => pick(&rules::REQUIEM_LINE_DAMAGE, level),
        AbilityId::MEAT_HOOK => pick(&rules::HOOK_DAMAGE, level),
        AbilityId::ROT => pick(&rules::ROT_DAMAGE_PER_SECOND, level),
        AbilityId::DISMEMBER => pick(&rules::DISMEMBER_DAMAGE_PER_SECOND, level),
        _ => 0,
    }
}

fn pick<T: Copy + Default>(values: &[T], level: u8) -> T {
    values
        .get(usize::from(level.saturating_sub(1)))
        .or_else(|| values.last())
        .copied()
        .unwrap_or_default()
}
fn integer(value: i32) -> Dec {
    Dec(value.saturating_mul(1000))
}
fn fixed(value: bota_proto::Fixed) -> Dec {
    Dec((i64::from(value.raw) * 1000 / 65536).clamp(i32::MIN.into(), i32::MAX.into()) as i32)
}
fn ticks(value: u32) -> Dec {
    Dec((u64::from(value) * 1000 / u64::from(rules::TICKS_PER_SECOND)).min(i32::MAX as u64) as i32)
}

/// A named numeric value that existing core metadata or a typed mechanic supplies.
/// Unknown fields remain absent rather than inventing data not present in the catalogue.
pub fn ability_value(id: AbilityId, level: u8, key: &str) -> Option<Dec> {
    let def = bota_core::ability_def(id)?;
    let common = match key {
        "AbilityManaCost" | "mana_cost" => Some(integer(bota_core::ability_mana_cost(id, level))),
        "AbilityCooldown" | "cooldown" => Some(ticks(bota_core::ability_cooldown(id, level))),
        "AbilityCastRange" | "cast_range" => Some(integer(def.range)),
        "AbilityCastPoint" | "cast_point" => Some(Dec(def.point.min(i32::MAX as u32) as i32)),
        "AbilityChannelTime" | "channel_time" => Some(ticks(def.duration)),
        _ => None,
    };
    common
        .or_else(|| NeutralAbility::of(id).and_then(|ability| neutral_value(ability, key, level)))
        .or_else(|| utility_value(id, key))
        .or_else(|| hero_value(id, key, level))
}

fn neutral_value(ability: NeutralAbility, key: &str, level: u8) -> Option<Dec> {
    let raw = match ability {
        NeutralAbility::StrengthOfTheImmortal => Some(&crate::roshan_data::ROSHAN_INHERENT_BUFFS),
        NeutralAbility::Devotion => Some(&crate::roshan_data::ROSHAN_DEVOTION),
        NeutralAbility::SpellBlock => Some(&crate::roshan_data::ROSHAN_SPELL_BLOCK),
        NeutralAbility::Bash => Some(&crate::roshan_data::ROSHAN_BASH),
        NeutralAbility::RoshanSlam => Some(&crate::roshan_data::ROSHAN_SLAM),
        NeutralAbility::RevengeRoar => Some(&crate::roshan_data::ROSHAN_REVENGEROAR),
        NeutralAbility::GrabAndThrow => Some(&crate::roshan_data::ROSHAN_GRAB),
        _ => None,
    };
    if let Some(value) = raw.and_then(|data| {
        data.values
            .iter()
            .find(|(name, _)| *name == key)
            .and_then(|(_, values)| {
                values
                    .get(usize::from(level.saturating_sub(1)))
                    .or_else(|| values.last())
            })
            .copied()
    }) {
        return Some(value);
    }
    match *ability.spell() {
        Spell::Cast { act, .. } => act_value(act, key, level),
        Spell::Aura(aura) => {
            if key == "radius" {
                Some(integer(aura.radius))
            } else {
                buff_value(aura.buff, key, level)
            }
        }
        Spell::Intrinsic { buff, aura, .. } => {
            if key == "radius" {
                aura.map(|aura| integer(aura.radius))
            } else {
                buff_value(buff, key, level)
            }
        }
        Spell::Block { buff, .. } => buff_value(buff, key, level),
        Spell::Storm {
            radius,
            near_radius,
            near_dps,
            far_dps,
            period_ms,
            buff,
            ..
        } => match key {
            "radius" => Some(integer(radius)),
            "near_radius" => Some(integer(near_radius)),
            "damage" | "damage_per_second" => Some(integer(near_dps)),
            "far_damage_per_second" => Some(integer(far_dps)),
            "interval" => Some(Dec(period_ms)),
            _ => buff_value(buff, key, level),
        },
        Spell::Strike(strike) => strike_value(strike, key, level),
        Spell::Throe(Throe::Rouse {
            radius, ms, buff, ..
        }) => match key {
            "radius" => Some(integer(radius)),
            "duration" => Some(Dec(ms)),
            _ => buff_value(buff, key, level),
        },
        Spell::Throe(Throe::Split {
            count,
            hp,
            damage,
            life_ms,
            radius,
            slow_pct,
            slow_ms,
            ..
        }) => match key {
            "count" => Some(integer(at_level(&count, level))),
            "health" => Some(integer(at_level(&hp, level))),
            "damage" => Some(integer(at_level(&damage, level))),
            "duration" => Some(Dec(life_ms)),
            "radius" => Some(integer(radius)),
            "slow_amount" => Some(integer(slow_pct)),
            "slow_duration" => Some(Dec(slow_ms)),
            _ => None,
        },
        Spell::Uncast | Spell::Idle | Spell::Upgrade | Spell::Piercing => None,
    }
}

fn act_value(act: Act, key: &str, level: u8) -> Option<Dec> {
    match act {
        Act::Stomp {
            radius,
            damage,
            hero_ms,
            other_ms,
            then,
            ..
        } => match key {
            "radius" => Some(integer(at_level(&radius, level))),
            "damage" => Some(integer(at_level(&damage, level))),
            "duration" | "stomp_duration" => Some(Dec(at_level(&hero_ms, level))),
            "creep_duration" => Some(Dec(at_level(&other_ms, level))),
            _ => match then {
                bota_core::Then::Wear { buff, .. } => buff_value(buff, key, level),
                bota_core::Then::Stun => None,
            },
        },
        Act::Bolt {
            speed,
            hero_damage,
            other_damage,
            ms,
            ..
        } => match key {
            "speed" => Some(integer(speed)),
            "damage" | "hero_damage" => Some(integer(hero_damage)),
            "creep_damage" => Some(integer(other_damage)),
            "duration" => Some(Dec(ms)),
            _ => None,
        },
        Act::Chain {
            damage,
            targets,
            radius,
            delay_ms,
            keep_pct,
        } => match key {
            "damage" => Some(integer(at_level(&damage, level))),
            "targets" => Some(integer(i32::from(targets))),
            "radius" => Some(integer(radius)),
            "jump_delay" => Some(Dec(delay_ms)),
            "damage_keep_pct" => Some(integer(at_level(&keep_pct, level))),
            _ => None,
        },
        Act::Summon {
            count,
            hp,
            damage,
            life_ms,
            ..
        } => match key {
            "count" => Some(integer(i32::from(count))),
            "health" => Some(integer(at_level(&hp, level))),
            "damage" => Some(integer(at_level(&damage, level))),
            "duration" => Some(Dec(life_ms)),
            _ => None,
        },
        Act::Enrage { buff, ms, .. } => {
            if key == "duration" {
                Some(Dec(ms))
            } else {
                buff_value(buff, key, level)
            }
        }
        Act::Smash {
            reach,
            radius,
            damage,
            current_pct,
            ms,
        } => match key {
            "range" => Some(integer(reach)),
            "radius" => Some(integer(at_level(&radius, level))),
            "damage" => Some(integer(at_level(&damage, level))),
            "current_health_pct" => Some(integer(current_pct)),
            "duration" => Some(Dec(ms)),
            _ => None,
        },
        Act::Spikes {
            lines,
            spread_deg,
            length,
            width,
            damage,
            ms,
        } => match key {
            "lines" => Some(integer(i32::from(lines))),
            "spread" => Some(integer(spread_deg)),
            "length" => Some(integer(length)),
            "width" => Some(integer(width)),
            "damage" => Some(integer(at_level(&damage, level))),
            "duration" => Some(Dec(ms)),
            _ => None,
        },
        Act::Bubble {
            absorb,
            ms,
            heal_pct,
            radius,
        } => match key {
            "damage_absorb" => Some(integer(at_level(&absorb, level))),
            "duration" => Some(Dec(ms)),
            "heal_pct" => Some(integer(heal_pct)),
            "radius" => Some(integer(radius)),
            _ => None,
        },
        Act::Push { distance, speed } => match key {
            "distance" => Some(integer(distance)),
            "speed" => Some(integer(speed)),
            _ => None,
        },
        Act::Purge { start_pct, ms } => match key {
            "slow_amount" => Some(integer(start_pct)),
            "duration" => Some(Dec(ms)),
            _ => None,
        },
        Act::Tornado { life_ms, .. } => (key == "duration").then_some(Dec(life_ms)),
        Act::Roar(spec) => match key {
            "radius" => Some(integer(spec.radius)),
            "damage" => Some(integer(spec.damage)),
            "windup_time" => Some(ticks(spec.windup)),
            "duration" => Some(ticks(spec.aura_duration)),
            _ => None,
        },
        Act::Grab(spec) => match key {
            "damage" => Some(integer(spec.damage)),
            "damage_per_combo" => Some(integer(spec.damage_per_throw)),
            "channel_time" => Some(ticks(spec.duration)),
            "interval" => Some(ticks(spec.contact_after)),
            "max_knockback_distance" => Some(integer(spec.distance)),
            "knockback_height" => Some(integer(spec.height)),
            "stun_duration" => Some(ticks(spec.stun)),
            _ => None,
        },
    }
}

fn strike_value(strike: Strike, key: &str, level: u8) -> Option<Dec> {
    match strike {
        Strike::Disarm { hits, ms, .. } => match key {
            "hits" => Some(integer(i32::from(hits))),
            "duration" => Some(Dec(at_level(&ms, level))),
            _ => None,
        },
        Strike::Break { ms } => (key == "duration").then_some(Dec(ms)),
        Strike::Lay {
            buff,
            hero_ms,
            other_ms,
            dps,
            ..
        } => match key {
            "damage" | "damage_per_second" => Some(integer(at_level(&dps, level))),
            "duration" => Some(Dec(hero_ms)),
            "creep_duration" => Some(Dec(other_ms)),
            _ => buff_value(buff, key, level),
        },
        Strike::Crit {
            chance_pct,
            mult_pct,
        } => match key {
            "crit_chance" => Some(integer(chance_pct)),
            "crit_multiplier" => Some(integer(at_level(&mult_pct, level))),
            _ => None,
        },
        Strike::Splash { radius, pct } => match key {
            "radius" => Some(integer(radius)),
            "damage_pct" => Some(integer(pct)),
            _ => None,
        },
        Strike::Petrify {
            below_pct, ms, dps, ..
        } => match key {
            "health_threshold" => Some(integer(below_pct)),
            "duration" => Some(Dec(at_level(&ms, level))),
            "damage" | "damage_per_second" => Some(integer(dps)),
            _ => None,
        },
        Strike::Bash {
            chance_pct,
            damage,
            stun_ms,
            ..
        } => match key {
            "bash_chance" => Some(integer(chance_pct)),
            "damage" | "bonus_damage" => Some(integer(at_level(&damage, level))),
            "stun_duration" => Some(Dec(at_level(&stun_ms, level))),
            _ => None,
        },
    }
}

fn buff_value(buff: Buff, key: &str, level: u8) -> Option<Dec> {
    let stat = match key {
        "bonus_movement_speed" | "movement_speed_pct" => Stat::MovePct,
        "bonus_attack_speed" => Stat::AttackSpeed,
        "bonus_armor" => Stat::Armor,
        "base_armor_pct" => Stat::BaseArmorPct,
        "bonus_damage" => Stat::Damage,
        "damage_pct" => Stat::DamagePct,
        "outgoing_damage_pct" => Stat::OutgoingPct,
        "bonus_health_pct" => Stat::MaxHpPct,
        "magic_resistance" => Stat::MagicResistPct,
        "magic_resistance_creeps" => Stat::MagicResistCreepPct,
        "health_regen" => Stat::HpRegen,
        "mana_regen" => Stat::ManaRegen,
        "regen_reduction_pct" => Stat::RegenCutPct,
        "lifesteal_pct" => Stat::LifestealPct,
        "magic_damage_taken_pct" => Stat::MagicTakenPct,
        "cooldown_reduction_pct" => Stat::CooldownPct,
        "accuracy_pct" => Stat::AccuracyPct,
        "slow_resistance" => Stat::SlowResistPct,
        "status_resistance" => Stat::StatusResistPct,
        "illusion_attack_reduction_pct" => Stat::IllusionAttackReductionPct,
        "incoming_damage_pct" => Stat::IncomingDamagePct,
        _ => return None,
    };
    if !buff.stats.iter().any(|(held, _)| *held == stat) {
        return None;
    }
    let value = buff.get(stat, level);
    Some(match stat {
        Stat::HpRegen => Dec(value.saturating_mul(100)),
        Stat::ManaRegen => Dec(value.saturating_mul(10)),
        _ => integer(value),
    })
}

fn utility_value(id: AbilityId, key: &str) -> Option<Dec> {
    match id {
        AbilityId::BURST => match key {
            "duration" => Some(ticks(rules::COURIER_BURST_TICKS)),
            "bonus_movement_speed" | "movement_speed_pct" => {
                Some(integer(rules::COURIER_BURST_PCT))
            }
            _ => None,
        },
        AbilityId::SHIELD => (key == "duration").then_some(ticks(rules::COURIER_SHIELD_TICKS)),
        AbilityId::TAKE_STASH => {
            (key == "stash_range").then_some(integer(rules::COURIER_STASH_RANGE))
        }
        AbilityId::DELIVER => {
            (key == "deliver_range").then_some(integer(rules::COURIER_HANDOFF_RANGE))
        }
        AbilityId::CAPTURE => {
            (key == "base_capture_time").then_some(ticks(bota_core::CAPTURE_TICKS))
        }
        AbilityId::LAMP_USE => match key {
            "active_duration" => Some(ticks(bota_core::LAMP_LIT_TICKS)),
            "inactive_duration" => Some(ticks(bota_core::LAMP_OFF_TICKS)),
            _ => None,
        },
        _ => None,
    }
}

fn hero_value(id: AbilityId, key: &str, level: u8) -> Option<Dec> {
    if let Some(index) = bota_core::heroes::shadow_fiend::SHADOW_FIEND_RAZES
        .iter()
        .position(|known| *known == id)
    {
        return match key {
            "damage" | "shadowraze_damage" => Some(integer(pick(&rules::RAZE_DAMAGE, level))),
            "shadowraze_radius" | "radius" => Some(integer(rules::RAZE_RADIUS)),
            "shadowraze_range" | "range" => Some(integer(rules::RAZE_DISTANCE[index])),
            "stack_bonus_damage" => Some(integer(pick(&rules::RAZE_STACK_DAMAGE, level))),
            "damage_per_soul" => Some(integer(rules::RAZE_DAMAGE_PER_SOUL)),
            "duration" => Some(ticks(rules::RAZE_DEBUFF_TICKS)),
            _ => None,
        };
    }
    match id {
        AbilityId::MEAT_HOOK => match key {
            "damage" | "hook_damage" => Some(integer(pick(&rules::HOOK_DAMAGE, level))),
            "hook_width" | "radius" => Some(integer(rules::HOOK_RADIUS)),
            "hook_speed" => Some(integer(rules::HOOK_SPEED)),
            "hook_distance" => Some(integer(rules::HOOK_RANGE)),
            _ => None,
        },
        AbilityId::ROT => match key {
            "damage" | "rot_damage" | "damage_per_second" => {
                Some(integer(pick(&rules::ROT_DAMAGE_PER_SECOND, level)))
            }
            "radius" | "rot_radius" => Some(integer(rules::ROT_RADIUS)),
            "rot_slow" => Some(integer(pick(&rules::ROT_SLOW_PCT, level))),
            _ => None,
        },
        AbilityId::DISMEMBER => match key {
            "damage" | "dismember_damage" | "damage_per_second" => {
                Some(integer(pick(&rules::DISMEMBER_DAMAGE_PER_SECOND, level)))
            }
            "strength_damage" => Some(fixed(pick(&rules::DISMEMBER_STRENGTH_DAMAGE, level))),
            "pull_speed" => Some(integer(rules::DISMEMBER_PULL_SPEED)),
            "pull_distance_limit" => Some(integer(rules::DISMEMBER_PULL_LIMIT)),
            _ => None,
        },
        AbilityId::FLESH_HEAP => match key {
            "flesh_heap_strength_buff_amount" => Some(fixed(rules::FLESH_HEAP_STRENGTH_PER_STACK)),
            "flesh_heap_range" | "radius" => Some(integer(rules::FLESH_HEAP_RANGE)),
            _ => None,
        },
        AbilityId::MEAT_SHIELD => match key {
            "damage_block" => Some(integer(pick(&rules::MEAT_SHIELD_BLOCK, level))),
            "duration" => Some(ticks(pick(&rules::MEAT_SHIELD_TICKS, level))),
            _ => None,
        },
        AbilityId::REQUIEM => match key {
            "damage" | "requiem_damage" => Some(integer(pick(&rules::REQUIEM_LINE_DAMAGE, level))),
            "requiem_radius" => Some(integer(rules::REQUIEM_LINE_DISTANCE)),
            "requiem_line_width_start" => Some(integer(rules::REQUIEM_LINE_WIDTH_START)),
            "requiem_line_width_end" => Some(integer(rules::REQUIEM_LINE_WIDTH_END)),
            "requiem_reduction_ms" => Some(integer(pick(&rules::REQUIEM_SLOW_PCT, level))),
            _ => None,
        },
        AbilityId::NECROMASTERY => match key {
            "necromastery_max_souls" | "max_souls" => Some(integer(rules::NECRO_SOUL_CAP as i32)),
            "necromastery_damage_per_soul" => Some(Dec(rules::SOUL_DAMAGE_TENTHS * 100)),
            _ => None,
        },
        AbilityId::PRESENCE => match key {
            "presence_radius" | "radius" => Some(integer(rules::PRESENCE_RADIUS)),
            "presence_armor_reduction" => Some(fixed(pick(&rules::PRESENCE_ARMOR, level))),
            _ => None,
        },
        AbilityId::FEAST_OF_SOULS => match key {
            "bonus_attack_speed" => Some(integer(pick(&rules::FEAST_ATTACK_SPEED, level))),
            "bonus_movement_speed" => Some(integer(pick(&rules::FEAST_MOVE_PCT, level))),
            "radius" => Some(integer(rules::FEAST_RADIUS)),
            "duration" => Some(ticks(rules::FEAST_TICKS)),
            "max_targets" => Some(integer(pick(&rules::FEAST_TARGETS, level) as i32)),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod courier_handoff_tests {
    use super::*;

    #[test]
    fn courier_deliver_range_reports_the_handoff_distance_without_a_cast_range() {
        assert_eq!(
            ability_value(AbilityId::DELIVER, 1, "deliver_range"),
            Some(Dec(400_000)),
            "Transfer Items projects its 7.41f handoff distance in world units"
        );
        assert_eq!(
            ability_value(AbilityId::DELIVER, 1, "AbilityCastRange"),
            Some(Dec(0)),
            "a no-target ability has no cast range to substitute for its handoff distance"
        );
        assert_eq!(
            ability_value(AbilityId::TAKE_STASH, 1, "deliver_range"),
            None,
            "a delivery-only value must not become a generic stash/pickup range"
        );
    }
}
