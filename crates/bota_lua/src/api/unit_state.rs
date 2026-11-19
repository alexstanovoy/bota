//! Thin control-state adapters over the receiving client's unit sheet.
use super::{context::ApiContext, *};
pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    let methods = context
        .state
        .borrow()
        .unit_methods
        .expect("installed unit methods");
    for name in ["IsSilenced", "IsRooted", "IsDisarmed"] {
        let context = context.clone();
        method(
            lua,
            methods,
            name,
            Rc::new(move |vm, args| read(&context, vm, args, name)),
        )?;
    }
    Ok(())
}
fn read(context: &ApiContext, vm: &mut Vm, args: Args, name: &str) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(8 + 2 * (snapshot.units.len() + snapshot.players.len()) as u64)?;
    if !state.askable(id) {
        state.decline(match name {
            "IsSilenced" => "IsSilenced_cannot_be_seen",
            "IsRooted" => "IsRooted_cannot_be_seen",
            _ => "IsDisarmed_cannot_be_seen",
        });
        return vm.results(&[Value::Boolean(false)]);
    }
    let dead = state
        .player(id)
        .is_some_and(|player| player.respawn_left > 0);
    let on = !dead
        && state.unit(id).is_some_and(|unit| match name {
            "IsSilenced" => bota_client_core::unit_state::is_silenced(unit),
            "IsRooted" => bota_client_core::unit_state::is_rooted(unit),
            "IsDisarmed" => bota_client_core::unit_state::is_disarmed(unit),
            _ => unreachable!(),
        });
    vm.results(&[Value::Boolean(on)])
}
