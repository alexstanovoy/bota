//! Team lane queries consume the host's validated script-decision cache.

use super::{
    context::{ApiContext, register_global},
    globals::bounded_integer,
};
use crate::{BindingError, state::Lane};
use luagee::{Args, Lua, LuaError, Value, Vm};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "GetPushLaneDesire", push)?;
    register_global(lua, context, "GetDefendLaneDesire", defend)
}

fn push(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    read(context, vm, args, true)
}

fn defend(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    read(context, vm, args, false)
}

fn read(context: &ApiContext, vm: &mut Vm, args: Args, pushing: bool) -> Result<usize, LuaError> {
    let engine = bounded_integer(vm, args, 0, 1, 3)?;
    let lane = Lane::from_engine(engine).ok_or_else(|| vm.error("unknown engine lane"))?;
    vm.charge_fuel(2)?;
    let state = context.state.borrow();
    let desires = if pushing {
        state.team_desires.push
    } else {
        state.team_desires.defend
    };
    vm.results(&[Value::Number(
        desires.map_or(0.0, |desires| desires.get(lane)),
    )])
}
