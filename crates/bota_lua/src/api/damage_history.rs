//! Lua conventions over the client's observed source timestamps.
//!
//! The reference answers only for a teammate that is bot-controlled; any other receiver gets a
//! logged error and `false` (the host counts it as a declined call). Its hero history is kept per
//! player id, so every body of that player (illusions included) shares one answer, and its window
//! is the strict float comparison `interval > age`.
use super::{
    context::{ApiContext, register_unit},
    players::declared_bot,
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
    let state = context.state.borrow();
    let snapshot = scanned(&state, vm)?;
    let answer = answer(
        &state,
        snapshot,
        id,
        seconds,
        DamageSource::Creep,
        "WasRecentlyDamagedByCreep_not_a_bot_teammate",
    );
    vm.results(&[Value::Boolean(answer)])
}
fn hero(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let attacker = context.unit_id(vm, args, 1)?;
    let seconds = vector::number(vm, args, 2)?;
    let state = context.state.borrow();
    let snapshot = scanned(&state, vm)?;
    // The reference keys by the attacker's player, so the scoreboard record answers even after
    // the body's track expired; any other body of the player (an illusion) names it by owner.
    let slot = state
        .player(attacker)
        .map(|player| player.slot)
        .or_else(|| {
            state
                .unit(attacker)
                .filter(|unit| unit.kind == UnitKind::Hero)
                .and_then(|unit| unit.owner)
        });
    let answer = slot.is_some_and(|slot| {
        answer(
            &state,
            snapshot,
            id,
            seconds,
            DamageSource::Hero(Some(slot)),
            "WasRecentlyDamagedByHero_not_a_bot_teammate",
        )
    });
    vm.results(&[Value::Boolean(answer)])
}
/// The view the scans below read, with their whole price paid before any of them runs.
fn scanned<'a>(state: &'a State, vm: &mut Vm) -> Result<&'a crate::state::Snapshot, LuaError> {
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(16 + snapshot.units.len() as u64 + snapshot.players.len() as u64)?;
    Ok(snapshot)
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
fn answer(
    state: &State,
    snapshot: &crate::state::Snapshot,
    id: bota_proto::EntityId,
    seconds: f64,
    source: DamageSource,
    declined: &'static str,
) -> bool {
    let teammate_bot = state
        .player(id)
        .is_some_and(|player| player.team == state.team && declared_bot(state, player.slot));
    if !teammate_bot {
        state.decline(declined);
        return false;
    }
    snapshot
        .units
        .iter()
        .find(|known| known.unit.id == id)
        .is_some_and(|known| {
            known
                .recent_damage
                .within(snapshot.tick, window(seconds), source)
        })
}
