//! Reference activity ids over the shared, filtered client activity model.

use super::{
    context::{ApiContext, register_unit},
    unit_names,
};
use crate::BindingError;
use bota_client_core::Activity;
use luagee::{Args, Lua, LuaError, Value, Vm};

// Activity ids established from the installed game, build 6975.
const CONSTANTS: &[(&str, i32)] = &[
    ("ACTIVITY_IDLE", 1500),
    ("ACTIVITY_IDLE_RARE", 1501),
    ("ACTIVITY_RUN", 1502),
    ("ACTIVITY_ATTACK", 1503),
    ("ACTIVITY_ATTACK2", 1504),
    ("ACTIVITY_DIE", 1506),
    ("ACTIVITY_FLAIL", 1508),
    ("ACTIVITY_DISABLED", 1509),
    ("ACTIVITY_CAST_ABILITY_1", 1510),
    ("ACTIVITY_CAST_ABILITY_2", 1511),
    ("ACTIVITY_CAST_ABILITY_3", 1512),
    ("ACTIVITY_CAST_ABILITY_4", 1513),
    ("ACTIVITY_CAST_ABILITY_5", 1514),
    ("ACTIVITY_CAST_ABILITY_6", 1515),
    ("ACTIVITY_CHANNEL_ABILITY_1", 1520),
    ("ACTIVITY_CHANNEL_ABILITY_2", 1521),
    ("ACTIVITY_CHANNEL_ABILITY_3", 1522),
    ("ACTIVITY_CHANNEL_ABILITY_4", 1523),
    ("ACTIVITY_CHANNEL_ABILITY_5", 1524),
    ("ACTIVITY_CHANNEL_ABILITY_6", 1525),
    ("ACTIVITY_SPAWN", 1534),
];

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    for &(name, value) in CONSTANTS {
        lua.set_global(name, Value::Number(f64::from(value)))?;
    }
    register_unit(lua, context, "GetAnimActivity", activity)?;
    register_unit(lua, context, "GetAnimCycle", cycle)
}
fn activity(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    read(context, vm, args, false)
}
fn cycle(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    read(context, vm, args, true)
}
fn read(context: &ApiContext, vm: &mut Vm, args: Args, cycle: bool) -> Result<usize, LuaError> {
    vm.charge_fuel(super::CALL_FUEL)?;
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel((2 * snapshot.units.len() + 2 * snapshot.players.len()) as u64)?;
    let known = snapshot.units.iter().find(|known| known.unit.id == id);
    if known.is_none() && state.player(id).is_none() {
        return Err(vm.error("unit is no longer known"));
    }
    let Some(known) = known.filter(|known| known.visible) else {
        return vm.results(&[Value::Number(-1.0)]);
    };
    let Some(profile) =
        unit_names::key(&state, id).and_then(|key| state.activity_profiles.get(&key).copied())
    else {
        return vm.results(&[Value::Number(-1.0)]);
    };
    let Some(observation) = known.activity.sample(profile) else {
        return vm.results(&[Value::Number(-1.0)]);
    };
    let value = if cycle {
        observation.cycle.map_or(-1.0, fixed_number)
    } else {
        f64::from(reference_id(vm, observation.activity)?)
    };
    vm.results(&[Value::Number(value)])
}
#[expect(
    clippy::float_arithmetic,
    reason = "the checked shared model cycle is a Q16 fraction converted to the API float"
)]
fn fixed_number(cycle: bota_proto::Fixed) -> f64 {
    f64::from(cycle.raw) / 65536.0
}
fn reference_id(vm: &mut Vm, activity: Activity) -> Result<i32, LuaError> {
    let mut slot = |slot: Option<bota_proto::AbilitySlot>, base: i32| {
        let slot = slot.map_or(0, |slot| slot.0);
        if slot >= 6 {
            Err(vm.error("activity ability slot has no reference mapping"))
        } else {
            Ok(base + i32::from(slot))
        }
    };
    match activity {
        Activity::Idle => Ok(1500),
        Activity::IdleRare => Ok(1501),
        Activity::Run => Ok(1502),
        Activity::Attack { alternate } => Ok(if alternate { 1504 } else { 1503 }),
        Activity::Cast { slot: s, .. } => slot(s, 1510),
        Activity::Channel { slot: s, .. } => slot(s, 1520),
        Activity::Flail => Ok(1508),
        Activity::Stunned => Ok(1509),
        Activity::Die => Ok(1506),
        Activity::Spawn => Ok(1534),
        Activity::Teleport => Err(vm.error("teleport activity has no reference mapping")),
    }
}
