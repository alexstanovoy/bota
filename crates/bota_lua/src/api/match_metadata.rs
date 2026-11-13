//! Match configuration facts; lane assignment is explicit for Solo Mid only.

use super::context::{ApiContext, register_global, register_unit};
use crate::BindingError;
use luagee::{Args, Lua, LuaError, Value, Vm};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "GetGameMode", game_mode)?;
    register_unit(lua, context, "GetAssignedLane", assigned_lane)
}

fn game_mode(context: &ApiContext, vm: &mut Vm, _args: Args) -> Result<usize, LuaError> {
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    if snapshot.map.id != bota_core::MAP2_ID {
        return Err(vm.error("the configured map has no engine game-mode mapping"));
    }
    vm.results(&[Value::Number(21.0)])
}

fn assigned_lane(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    let player = state.player(id);
    if player.is_none() && state.unit(id).is_none() {
        return Err(vm.error("unit is no longer known"));
    }
    // Solo Mid assigns roster heroes to mid. Other maps have no host lane plan.
    let lane = if snapshot.map.id == bota_core::MAP2_ID && player.is_some() {
        2.0
    } else {
        0.0
    };
    vm.results(&[Value::Number(lane)])
}
