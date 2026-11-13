//! The viewer's observed Glyph timer and an ordinary team Glyph request.
use super::{
    context::{ApiContext, register_global, register_unit},
    globals::seconds,
};
use crate::BindingError;
use bota_proto::Order;
use luagee::{Args, Lua, LuaError, Value, Vm};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "GetGlyphCooldown", cooldown)?;
    register_unit(lua, context, "ActionImmediate_Glyph", activate)
}
fn cooldown(context: &ApiContext, vm: &mut Vm, _args: Args) -> Result<usize, LuaError> {
    vm.charge_fuel(super::CALL_FUEL)?;
    let state = context.state.borrow();
    let ticks = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.glyph_cooldown)
        .ok_or_else(|| vm.error("glyph cooldown is not observed"))?;
    vm.results(&[Value::Number(seconds(ticks))])
}
fn activate(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    vm.charge_fuel(super::CALL_FUEL)?;
    let id = context.receiver(vm, args)?;
    context.issue_hero(vm, id, Order::Glyph)?;
    Ok(0)
}
