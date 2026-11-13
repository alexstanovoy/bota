//! Three complete examples for API families: a global, a filtered getter and an action.
use super::context::{ApiContext, register_global, register_unit};
use crate::{BindingError, vector};
use luagee::{Args, Lua, LuaError, Value, Vm};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "GetBot", get_bot)?;
    register_unit(lua, context, "GetHealth", get_health)?;
    register_unit(lua, context, "GetLocation", get_location)?;
    register_unit(lua, context, "GetOrigin", get_location)?;
    register_unit(lua, context, "Action_MoveToLocation", move_to_location)
}

fn get_location(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let position = context
        .state
        .borrow()
        .unit(id)
        .map(|unit| unit.pos)
        .ok_or_else(|| vm.error("unit is no longer known"))?;
    let value = context.vectors.location(vm, position)?;
    vm.results(&[value])
}

fn get_bot(context: &ApiContext, vm: &mut Vm, _args: Args) -> Result<usize, LuaError> {
    let id = context.state.borrow().own_id();
    let value = context.unit_value(vm, id)?;
    vm.results(&[value])
}

fn get_health(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let health = {
        let state = context.state.borrow();
        let dead = state.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .players
                .iter()
                .any(|player| player.unit == Some(id) && player.respawn_left > 0)
        });
        if dead {
            0
        } else {
            state.unit(id).map_or(0, |unit| unit.hp)
        }
    };
    vm.results(&[Value::Number(f64::from(health))])
}

fn move_to_location(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let receiver = context.receiver(vm, args)?;
    let position = vector::position(vm, vm.arg(args, 1))?;
    context.issue_move(vm, receiver, position)?;
    Ok(0)
}
