//! Incoming homing missiles from the current filtered snapshot.

use super::{
    abilities,
    context::{ApiContext, register_unit},
    players,
};
use crate::{BindingError, state::State};
use bota_proto::{EntityId, Inflictor, ItemSlot, ProjectileView};
use luagee::{Args, Lua, LuaError, TableId, Value, Vm};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_unit(lua, context, "GetIncomingTrackingProjectiles", incoming)
}
fn incoming(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let mut state = context.state.borrow_mut();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(
        16 + 2 * snapshot.units.len() as u64
            + 2 * snapshot.players.len() as u64
            + 2 * snapshot.projectiles.len() as u64,
    )?;
    let known = snapshot.units.iter().find(|known| known.unit.id == id);
    if known.is_none() && state.player(id).is_none() {
        return Err(vm.error("unit is no longer known"));
    }
    if known.is_none_or(|known| !known.visible) {
        return vm.results(&[Value::Nil]);
    }
    // Copy only filtered records after charging the bounded scan. No remembered missiles.
    let missiles: Vec<_> = snapshot
        .projectiles
        .iter()
        .filter(|missile| missile.target == Some(id))
        .copied()
        .collect();
    let table = vm.new_table()?;
    for (index, missile) in missiles.into_iter().enumerate() {
        let row = row(context, vm, &mut state, missile)?;
        vm.raw_set(table, Value::Number((index + 1) as f64), Value::Table(row))?;
    }
    vm.results(&[Value::Table(table)])
}
fn row(
    context: &ApiContext,
    vm: &mut Vm,
    state: &mut State,
    missile: ProjectileView,
) -> Result<TableId, LuaError> {
    let snapshot = state.snapshot.as_ref().expect("checked current view");
    let searches = match missile.inflictor {
        Some(Inflictor::Item(_)) => 36,
        Some(Inflictor::Ability(_)) => 12,
        None => 2,
    };
    vm.charge_fuel(24 + searches * (snapshot.units.len() + snapshot.players.len()) as u64)?;
    let table = vm.new_table()?;
    let location = context.vectors.location(vm, missile.pos)?;
    field(vm, table, b"location", location)?;
    let caster = state.handle(vm, missile.source)?;
    field(vm, table, b"caster", caster)?;
    let slot = missile.source.and_then(|id| {
        state
            .player(id)
            .map(|player| player.slot)
            .or_else(|| state.unit(id).and_then(|unit| unit.owner))
    });
    let player = slot
        .and_then(|slot| players::engine_id(state.snapshot.as_ref().expect("checked view"), slot))
        .unwrap_or(-1);
    field(vm, table, b"playerid", Value::Number(f64::from(player)))?;
    let identity = missile
        .source
        .zip(missile.inflictor)
        .and_then(|(source, inflictor)| inflictor_identity(state, source, inflictor));
    let ability = abilities::handle(vm, state, identity)?;
    field(vm, table, b"ability", ability)?;
    let attack = missile.inflictor.is_none();
    field(vm, table, b"is_attack", Value::Boolean(attack))?;
    // The simulation disjoints attack missiles; spell missiles keep homing.
    field(vm, table, b"is_dodgeable", Value::Boolean(attack))?;
    Ok(table)
}
fn inflictor_identity(
    state: &State,
    source: EntityId,
    inflictor: Inflictor,
) -> Option<abilities::Identity> {
    match inflictor {
        Inflictor::Ability(id) => {
            abilities::lookup_ability(state, source, bota_core::ability_def(id)?.key.as_bytes())
        }
        Inflictor::Item(id) => (0..=15).find_map(|slot| {
            let slot = ItemSlot(slot);
            (state.item(source, slot)?.id == id).then_some(abilities::Identity::Item {
                unit: source,
                slot,
                id,
            })
        }),
    }
}
fn field(vm: &mut Vm, table: TableId, key: &[u8], value: Value) -> Result<(), LuaError> {
    let key = vm.new_string(key)?;
    vm.raw_set(table, key, value)
}
