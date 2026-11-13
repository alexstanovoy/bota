//! Small numeric globals with checked engine-float operands.

use super::context::{ApiContext, register_global};
use crate::{BindingError, vector};
use luagee::{Args, Lua, LuaError, Value, Vm};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "Min", minimum)?;
    register_global(lua, context, "Max", maximum)
}

fn operand(vm: &mut Vm, args: Args, index: usize) -> Result<f64, LuaError> {
    let value = vector::number(vm, args, index)?;
    if value.abs() > f64::from(f32::MAX) {
        return Err(vm.arg_error(args, index, "engine float within bounds expected"));
    }
    Ok(value)
}

fn minimum(_context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let left = operand(vm, args, 0)?;
    let right = operand(vm, args, 1)?;
    vm.results(&[Value::Number(if left < right { left } else { right })])
}

fn maximum(_context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let left = operand(vm, args, 0)?;
    let right = operand(vm, args, 1)?;
    vm.results(&[Value::Number(if left > right { left } else { right })])
}
