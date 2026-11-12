//! Position-independent damage projections from observed stats and core spell data.

#![allow(
    clippy::float_arithmetic,
    reason = "the API projects damage over a finite fractional duration"
)]

use bota_client_core::{AbilityObservation, DamageCadence};
use bota_core::{UseSpec, rules};
use bota_proto::{AbilityView, DamageKind, StatusFlags, UnitView};

use super::{
    context::{ApiContext, register_unit},
    *,
};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_unit(lua, context, "GetEstimatedDamageToTarget", estimate)
}

fn estimate(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let actor_id = context.receiver(vm, args)?;
    let target_id = context.unit_id(vm, args, 2)?;
    let available = match vm.arg(args, 1) {
        Value::Boolean(value) => value,
        _ => return Err(vm.arg_error(args, 1, "boolean expected")),
    };
    let seconds = vector::number(vm, args, 3)?.max(0.0);
    let mask = super::globals::integer(vm, args, 4)?;
    let state = context.state.borrow();
    let actor = state
        .unit(actor_id)
        .ok_or_else(|| vm.error("unit is no longer known"))?;
    let target = state
        .unit(target_id)
        .ok_or_else(|| vm.error("unit is no longer known"))?;
    vm.charge_fuel(
        32 + (actor.abilities.len() + actor.items.len() + actor.effects.len()) as u64 * 16
            + actor.abilities.len() as u64 * actor.effects.len() as u64 * 2,
    )?;
    let damage = if seconds == 0.0 || target.statuses.bits & StatusFlags::INVULNERABLE != 0 {
        0.0
    } else {
        let available = available && actor.team == state.team;
        let attacks = if mask & 1 != 0 {
            attack_damage(vm, actor, target, seconds)?
        } else {
            0.0
        };
        let snapshot = state
            .snapshot
            .as_ref()
            .ok_or_else(|| vm.error("a current view is required"))?;
        let (spells, mana) = spell_damage(
            AbilityObservation {
                actor,
                target,
                clock: snapshot.game_tick,
                schedule: rules::Schedule::of(snapshot.horn),
            },
            seconds,
            mask,
            available,
        );
        attacks + spells + item_damage(actor, target, seconds, mask, available, mana)
    };
    if !damage.is_finite() {
        return Err(vm.error("damage estimate is not finite"));
    }
    vm.results(&[Value::Number(
        damage.clamp(0.0, f64::from(i32::MAX)).trunc(),
    )])
}

fn attack_damage(vm: &mut Vm, actor: &UnitView, target: &UnitView, seconds: f64) -> Result<f64, LuaError> {
    let class = bota_core::attack_class_pct(actor.kind, target.kind);
    let amount = (i64::from(actor.attack_damage.max(0)) * i64::from(class) / 100)
        .min(i64::from(i32::MAX)) as i32;
    let hit = crate::numeric::physical_damage(amount, target.armor)
        .ok_or_else(|| vm.error("received attack statistics exceed projection range"))?;
    let evasion = f64::from(target.evasion.raw).clamp(0.0, 65536.0) / 65536.0;
    Ok(f64::from(hit) * (1.0 - evasion) * seconds * 1000.0 / f64::from(actor.attack_time.max(1)))
}

fn accepts(mask: i64, kind: DamageKind) -> bool {
    let bit = match kind {
        DamageKind::Physical => 1,
        DamageKind::Magical => 2,
        DamageKind::Pure => 4,
    };
    mask & bit != 0
}

fn spell_reaches(definition: &bota_core::AbilityDef, target: &UnitView) -> bool {
    if target.statuses.bits & StatusFlags::MAGIC_IMMUNE == 0 {
        return true;
    }
    match definition.pierces {
        bota_core::Pierces::No => false,
        bota_core::Pierces::Yes => true,
        bota_core::Pierces::NotCouriers => target.kind != bota_proto::UnitKind::Courier,
    }
}

fn mitigated(amount: f64, kind: DamageKind, target: &UnitView) -> f64 {
    let reference = bota_core::mitigate(
        1_000_000,
        kind,
        target.armor,
        bota_core::magic_resist_bp(target.magic_resist),
    );
    amount * f64::from(reference) / 1_000_000.0
}

fn spell_damage(
    observation: AbilityObservation<'_>,
    seconds: f64,
    mask: i64,
    available: bool,
) -> (f64, i32) {
    let actor = observation.actor;
    let target = observation.target;
    let mut damage = 0.0;
    let mut mana = actor.mana.max(0);
    for ability in &actor.abilities {
        if ability.level == 0
            || available && (ability.cooldown_left > 0 || ability.mana_cost > mana)
        {
            continue;
        }
        let Some(definition) = bota_core::ability_def(ability.id) else {
            continue;
        };
        if !bota_core::kind_def(target.kind).struck_by_area_spells {
            continue;
        }
        let duration = (seconds - f64::from(definition.point) / 1000.0).max(0.0);
        if duration == 0.0 {
            continue;
        }
        let Some(projection) = bota_client_core::ability_damage_projection(
            ability.id,
            ability.level,
            duration,
            observation,
        ) else {
            continue;
        };
        if !accepts(mask, projection.kind)
            || projection.cadence != DamageCadence::Attack && !spell_reaches(definition, target)
        {
            continue;
        }
        let casts = match projection.cadence {
            DamageCadence::Cast => cast_count(ability, duration, available, mana),
            DamageCadence::Continuous => 1.0,
            DamageCadence::Attack => duration * 1000.0 / f64::from(actor.attack_time.max(1)),
        };
        if available {
            mana = mana.saturating_sub(
                (casts * f64::from(ability.mana_cost.max(0))).min(f64::from(i32::MAX)) as i32,
            );
        }
        damage += mitigated(projection.amount * casts, projection.kind, target);
    }
    (damage, mana)
}

fn cast_count(ability: &AbilityView, duration: f64, available: bool, mana: i32) -> f64 {
    let cooldown = f64::from(bota_core::ability_cooldown(ability.id, ability.level))
        / f64::from(rules::TICKS_PER_SECOND);
    let casts = if cooldown > 0.0 {
        1.0 + (duration / cooldown).floor()
    } else {
        1.0
    };
    if available && ability.mana_cost > 0 {
        casts.min(f64::from(mana / ability.mana_cost))
    } else {
        casts
    }
}

fn item_damage(
    actor: &UnitView,
    target: &UnitView,
    seconds: f64,
    mask: i64,
    available: bool,
    mut mana: i32,
) -> f64 {
    if !accepts(mask, DamageKind::Magical)
        || target.statuses.bits & StatusFlags::MAGIC_IMMUNE != 0
        || !bota_core::kind_def(target.kind).struck_by_area_spells
    {
        return 0.0;
    }
    actor
        .items
        .iter()
        .take(bota_proto::limits::INVENTORY_SLOTS)
        .flatten()
        .filter_map(|item| {
            if available && (item.cooldown_left > 0 || item.mute_left > 0 || item.mana_cost > mana)
            {
                return None;
            }
            let definition = bota_core::item_def(item.id)?;
            let UseSpec::Grenade(grenade) = definition.effect else {
                return None;
            };
            let duration = seconds - f64::from(definition.point) / 1000.0;
            if duration <= 0.0 || item.charges == Some(0) {
                return None;
            }
            if available {
                mana = mana.saturating_sub(item.mana_cost.max(0));
            }
            let burn = duration.min(f64::from(grenade.debuff_ticks) / 30.0).floor();
            Some(mitigated(
                f64::from(grenade.impact_damage) + f64::from(grenade.damage_per_second) * burn,
                DamageKind::Magical,
                target,
            ))
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bota_proto::AbilityId;
    mod support {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
    }

    fn project(id: AbilityId, kind: bota_proto::UnitKind, immune: bool) -> f64 {
        let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let mut actor = sessions[0].own_hero().expect("hero").clone();
        let mut ability = actor.abilities[0];
        ability.id = id;
        ability.level = 1;
        ability.cooldown_left = 0;
        actor.abilities = vec![ability];
        let mut target = actor.clone();
        target.kind = kind;
        target.statuses.bits = if immune { StatusFlags::MAGIC_IMMUNE } else { 0 };
        spell_damage(
            AbilityObservation {
                actor: &actor,
                target: &target,
                clock: 0,
                schedule: rules::Schedule::of(0),
            },
            3.0,
            7,
            false,
        )
        .0
    }

    #[test]
    fn dismember_estimates_piercing_damage_through_magic_immunity() {
        assert!(project(AbilityId::DISMEMBER, bota_proto::UnitKind::Hero, true) > 0.0);
    }

    #[test]
    fn hook_estimates_exclude_immune_couriers_but_include_unprotected_couriers() {
        assert_eq!(
            project(AbilityId::MEAT_HOOK, bota_proto::UnitKind::Courier, true),
            0.0
        );
        assert!(project(AbilityId::MEAT_HOOK, bota_proto::UnitKind::Courier, false) > 0.0);
    }
}
