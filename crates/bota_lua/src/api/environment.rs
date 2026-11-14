//! Public map locations and the receiving team's courier roster.
use super::{
    context::{ApiContext, register_global, register_unit},
    globals::{bounded_integer, integer, string},
};
use crate::BindingError;
use bota_proto::UnitKind;
use luagee::{Args, Lua, LuaError, Value, Vm};
pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    for (name, value) in [
        ("SHOP_HOME", 0),
        ("SHOP_SIDE", 1),
        ("SHOP_SECRET", 2),
        ("SHOP_SIDE2", 4),
        ("SHOP_SECRET2", 5),
        ("GAME_STATE_HERO_SELECTION", 2),
        ("GAME_STATE_PRE_GAME", 4),
        ("GAME_STATE_GAME_IN_PROGRESS", 5),
        ("GAME_STATE_POST_GAME", 6),
    ] {
        lua.set_global(name, Value::Number(f64::from(value)))?;
    }
    register_global(lua, context, "GetShopLocation", shop_location)?;
    register_unit(lua, context, "DistanceFromSideShop", side_distance)?;
    register_global(lua, context, "GetNumCouriers", courier_count)?;
    register_global(lua, context, "GetCourier", courier)?;
    register_global(lua, context, "GetGameState", game_state)?;
    register_unit(lua, context, "NumQueuedActions", queued_actions)?;
    register_global(lua, context, "IsItemPurchasedFromSideShop", side_item)
}
fn shop_location(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    vm.charge_fuel(super::CALL_FUEL)?;
    let team = bounded_integer(vm, args, 0, 2, 3)?;
    let shop = bounded_integer(vm, args, 1, 0, 5)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    let position = match shop {
        0 => Some(snapshot.map.fountains[usize::from(team == 3)]),
        2 => snapshot.map.secret_shops.first().map(|shop| shop.pos),
        5 => snapshot.map.secret_shops.get(1).map(|shop| shop.pos),
        _ => None,
    };
    let value = if let Some(position) = position {
        context.vectors.location(vm, position)?
    } else {
        context.vectors.value(vm, [0.0; 3])?
    };
    vm.results(&[value])
}
fn side_distance(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    vm.charge_fuel(super::CALL_FUEL)?;
    let state = context.state.borrow();
    if state.position(id).is_none() {
        return Err(vm.error("unit is no longer known"));
    }
    // Established from the installed game, build 6975: an absent shop's integer distance.
    vm.results(&[Value::Number(f64::from(i32::MIN))])
}
fn courier_count(context: &ApiContext, vm: &mut Vm, _args: Args) -> Result<usize, LuaError> {
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(super::CALL_FUEL + snapshot.units.len() as u64)?;
    let count = snapshot
        .units
        .iter()
        .filter(|known| {
            known.visible && known.unit.kind == UnitKind::Courier && known.unit.team == state.team
        })
        .count();
    vm.results(&[Value::Number(count as f64)])
}
fn courier(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let index = integer(vm, args, 0)?;
    let mut state = context.state.borrow_mut();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(super::CALL_FUEL + snapshot.units.len() as u64)?;
    let id = usize::try_from(index)
        .ok()
        .and_then(|index| {
            snapshot
                .units
                .iter()
                .filter(|known| {
                    known.visible
                        && known.unit.kind == UnitKind::Courier
                        && known.unit.team == state.team
                })
                .nth(index)
        })
        .map(|known| known.unit.id);
    let value = state.handle(vm, id)?;
    vm.results(&[value])
}
fn game_state(context: &ApiContext, vm: &mut Vm, _args: Args) -> Result<usize, LuaError> {
    vm.charge_fuel(super::CALL_FUEL)?;
    let state = context.state.borrow();
    let value = if let Some(snapshot) = &state.snapshot {
        match snapshot.ended {
            Some(bota_client_core::SessionEnd::Completed) => 6,
            Some(bota_client_core::SessionEnd::Aborted) => 7,
            None => {
                if snapshot.game_tick < snapshot.horn {
                    4
                } else {
                    5
                }
            }
        }
    } else if state.selecting {
        2
    } else if state.selection_finished {
        3
    } else {
        0
    };
    vm.results(&[Value::Number(f64::from(value))])
}
fn side_item(_context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    vm.charge_fuel(super::CALL_FUEL)?;
    let _key = string(vm, args, 0)?;
    vm.results(&[Value::Boolean(false)])
}

fn queued_actions(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    vm.charge_fuel(super::CALL_FUEL)?;
    let state = context.state.borrow();
    let count = if state.own_id() != Some(id) {
        -1.0
    } else if state
        .player(id)
        .is_some_and(|player| player.respawn_left > 0)
    {
        0.0
    } else {
        state.unit(id).map_or(0.0, |unit| unit.queue.len() as f64)
    };
    vm.results(&[Value::Number(count)])
}
