//! Ordinary inventory and queued movement requests; the server enforces legality.
use super::{
    context::{ApiContext, register_unit},
    globals::bounded_integer,
    *,
};
use bota_proto::{ItemSlot, Order, Target};
pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_unit(lua, context, "ActionImmediate_SwapItems", swap)?;
    register_unit(lua, context, "ActionQueue_MoveToLocation", queue_move)?;
    register_unit(lua, context, "Action_MovePath", path)
}
fn swap(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let from = bounded_integer(vm, args, 1, 0, i32::from(ItemSlot::TOWN_PORTAL.0))?;
    let to = bounded_integer(vm, args, 2, 0, i32::from(ItemSlot::TOWN_PORTAL.0))?;
    vm.charge_fuel(8)?;
    context.issue_hero(
        vm,
        id,
        Order::Swap {
            from: ItemSlot(from as u8),
            to: ItemSlot(to as u8),
        },
    )?;
    Ok(0)
}
fn queue_move(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let at = vector::position(vm, vm.arg(args, 1))?;
    let mut state = context.state.borrow_mut();
    vm.charge_fuel(8)?;
    if Some(id) != state.own_id() {
        return Err(vm.error("action requires the active bot"));
    }
    state.request_action(
        vm,
        id,
        bota_client_core::bot_actions::BotAction {
            kind: bota_client_core::bot_actions::BotActionType::MoveTo,
            order: Order::Move {
                target: Target::Pos(at),
            },
            once: false,
            effect: None,
        },
        bota_client_core::bot_actions::BotActionMode::Queue,
    )?;
    Ok(0)
}
fn path(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let Value::Table(table) = vm.arg(args, 1) else {
        return Err(vm.arg_error(args, 1, "waypoint table expected"));
    };
    let bound = bota_core::rules::MAX_QUEUED_ORDERS;
    vm.charge_fuel(16 + bound as u64 * 8)?;
    let mut points = Vec::with_capacity(bound);
    let mut ended = false;
    for at in 1..=bound + 1 {
        let value = vm.raw_get(table, Value::Number(at as f64))?;
        if value == Value::Nil {
            ended = true;
            continue;
        }
        if ended {
            return Err(vm.arg_error(args, 1, "waypoint table has a hole"));
        }
        if at > bound {
            return Err(vm.error("movement path exceeds the waypoint limit"));
        }
        points.push(vector::position(vm, value)?);
    }
    let orders = bota_client_core::movement_path_orders(&points)
        .ok_or_else(|| vm.error("movement path exceeds the waypoint limit"))?;
    let mut state = context.state.borrow_mut();
    if Some(id) != state.own_id() {
        return Err(vm.error("action requires the active bot"));
    }
    let actions: Vec<_> = orders
        .into_iter()
        .map(|issued| bota_client_core::bot_actions::BotAction {
            kind: bota_client_core::bot_actions::BotActionType::MoveTo,
            order: issued.order,
            once: false,
            effect: None,
        })
        .collect();
    state.request_actions(
        vm,
        id,
        &actions,
        bota_client_core::bot_actions::BotActionMode::Replace,
    )?;
    Ok(0)
}
