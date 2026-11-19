//! Bounded read of the seat's shared observation window.
use super::context::{ApiContext, register_unit};
use crate::BindingError;
use luagee::{Args, Lua, LuaError, Value, Vm};
pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_unit(lua, context, "GetMovementDirectionStability", read)
}
fn read(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    let rate = snapshot.statics.metadata().tick_rate;
    let window = bota_client_core::DirectionHistory::window_positions(rate)
        .unwrap_or(bota_client_core::DIRECTION_HISTORY_CAPACITY);
    vm.charge_fuel(8 + snapshot.units.len() as u64 + window as u64 * 32)?;
    let value = snapshot
        .units
        .iter()
        .find(|known| known.unit.id == id && known.visible)
        .and_then(|known| known.direction_history.stability(rate));
    if value.is_none() {
        state.decline("GetMovementDirectionStability_unobserved_window");
    }
    vm.results(&[value.map_or(Value::Nil, |value| Value::Number(f64::from(value)))])
}
