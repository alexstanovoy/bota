//! Lua conventions over the client's observed source timestamps.
use super::{
    context::{ApiContext, register_unit},
    *,
};
use bota_client_core::DamageSource;
use bota_proto::UnitKind;
pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_unit(lua, context, "WasRecentlyDamagedByCreep", creep)?;
    register_unit(lua, context, "WasRecentlyDamagedByHero", hero)
}
fn creep(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let seconds = vector::number(vm, args, 1)?;
    query(context, vm, id, seconds, DamageSource::Creep)
}
fn hero(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let source = context.unit_id(vm, args, 1)?;
    let seconds = vector::number(vm, args, 2)?;
    let state = context.state.borrow();
    let source = state
        .unit(source)
        .filter(|unit| unit.kind == UnitKind::Hero)
        .and_then(|unit| unit.owner);
    drop(state);
    match source {
        Some(slot) => query(context, vm, id, seconds, DamageSource::Hero(Some(slot))),
        None => {
            vm.charge_fuel(16)?;
            vm.results(&[Value::Boolean(false)])
        }
    }
}
#[expect(
    clippy::float_arithmetic,
    reason = "finite API seconds map to the first exclusive simulation tick"
)]
fn window(seconds: f64) -> u32 {
    (seconds.max(0.0) * f64::from(bota_core::rules::TICKS_PER_SECOND))
        .ceil()
        .min(f64::from(u32::MAX)) as u32
}
fn query(
    context: &ApiContext,
    vm: &mut Vm,
    id: bota_proto::EntityId,
    seconds: f64,
    source: DamageSource,
) -> Result<usize, LuaError> {
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(16 + snapshot.units.len() as u64 + snapshot.players.len() as u64)?;
    let controlled = state.player(id).is_some_and(|player| {
        player.team == state.team && state.seats.iter().any(|seat| seat.slot == player.slot)
    });
    let result = controlled
        && snapshot
            .units
            .iter()
            .find(|known| known.unit.id == id)
            .is_some_and(|known| {
                known
                    .recent_damage
                    .within(snapshot.tick, window(seconds), source)
            });
    vm.results(&[Value::Boolean(result)])
}
