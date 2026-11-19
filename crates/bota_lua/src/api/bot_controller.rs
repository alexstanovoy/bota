//! Names, argument checks and bounded work for the shared client controller.
use super::{
    context::{ApiContext, register_unit},
    globals::bounded_integer,
};
use crate::BindingError;
use bota_client_core::bot_actions::{BotAction, BotActionMode, BotActionType};
use bota_proto::{Order, Target};
use luagee::{Args, Lua, LuaError, Value, Vm};
pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    for name in [
        "GetCurrentActionType",
        "GetQueuedActionType",
        "NumQueuedActions",
    ] {
        let context = context.clone();
        let methods = context.state.borrow().unit_methods.expect("unit methods");
        super::method(
            lua,
            methods,
            name,
            std::rc::Rc::new(move |vm, args| read(&context, vm, args, name)),
        )?;
    }
    register_unit(lua, context, "ActionQueue_AttackUnit", queue_attack)
}
fn number(kind: BotActionType) -> f64 {
    match kind {
        BotActionType::Idle => 1.0,
        BotActionType::MoveTo => 2.0,
        BotActionType::MoveDirectly => 3.0,
        BotActionType::Attack => 4.0,
        BotActionType::AttackMove => 5.0,
        BotActionType::UseAbility => 6.0,
    }
}
fn read(context: &ApiContext, vm: &mut Vm, args: Args, name: &str) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    vm.charge_fuel(
        8 + state
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.units.len()) as u64,
    )?;
    let body = state.controller_body(id);
    let value = if let Some(body) = body {
        let controller = &state.bots[state.bot_index()].actions[body];
        match name {
            "GetCurrentActionType" => number(controller.current_type()),
            "NumQueuedActions" => controller.queued_len() as f64,
            _ => {
                let at = bounded_integer(vm, args, 1, i32::MIN, i32::MAX)?;
                usize::try_from(at)
                    .ok()
                    .and_then(|at| controller.queued_type(at))
                    .map_or(-1.0, number)
            }
        }
    } else if name == "GetCurrentActionType" {
        0.0
    } else {
        -1.0
    };
    vm.results(&[Value::Number(value)])
}
fn queue_attack(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let target = context.unit_id(vm, args, 1)?;
    let once = match vm.arg(args, 2) {
        Value::Nil => false,
        Value::Boolean(once) => once,
        _ => return Err(vm.arg_error(args, 2, "boolean expected")),
    };
    context.state.borrow_mut().request_action(
        vm,
        id,
        BotAction {
            kind: BotActionType::Attack,
            order: Order::Attack {
                target: Target::Unit(target),
            },
            once,
            effect: None,
        },
        BotActionMode::Queue,
    )?;
    Ok(0)
}
