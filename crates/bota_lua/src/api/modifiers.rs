//! Conditions and modifier metadata from received unit observations only.

use super::{
    context::ApiContext,
    globals::{bounded_integer, seconds, string},
    *,
};
use bota_proto::{EffectId, StatusFlags, UnitKind, UnitView};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    for name in [
        "HasModifier",
        "NumModifiers",
        "GetModifierByName",
        "GetModifierName",
        "GetModifierRemainingDuration",
        "GetModifierStackCount",
        "IsIllusion",
        "IsStunned",
        "IsInvisible",
        "IsCastingAbility",
        "IsCreep",
        "IsTower",
    ] {
        let context = context.clone();
        let methods = context
            .state
            .borrow()
            .unit_methods
            .ok_or(BindingError::Configuration(
                "unit methods are not initialized",
            ))?;
        method(
            lua,
            methods,
            name,
            Rc::new(move |vm, args| read(&context, vm, args, name)),
        )?;
    }
    Ok(())
}

fn named_index(vm: &mut Vm, args: Args, unit: &UnitView) -> Result<Option<usize>, LuaError> {
    if !matches!(vm.arg(args, 1), Value::String(_)) {
        return Err(vm.arg_error(args, 1, "modifier name expected"));
    }
    let key = string(vm, args, 1)?;
    Ok(unit.effects.iter().position(|effect| {
        bota_core::effect_def(effect.id).is_some_and(|definition| definition.key.as_bytes() == key)
    }))
}

fn read(context: &ApiContext, vm: &mut Vm, args: Args, name: &str) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    let unit = state
        .unit(id)
        .ok_or_else(|| vm.error("unit is no longer known"))?;
    vm.charge_fuel(16 + unit.effects.len() as u64 * 8)?;
    let value = match name {
        "HasModifier" => Value::Boolean(named_index(vm, args, unit)?.is_some()),
        "GetModifierByName" => {
            Value::Number(named_index(vm, args, unit)?.map_or(-1.0, |index| index as f64))
        }
        "NumModifiers" => Value::Number(unit.effects.len() as f64),
        "IsIllusion" => Value::Boolean(
            unit.effects
                .iter()
                .any(|effect| effect.id == EffectId::ILLUSION),
        ),
        "IsStunned" => Value::Boolean(unit.statuses.bits & StatusFlags::STUNNED != 0),
        "IsInvisible" => Value::Boolean(unit.statuses.bits & StatusFlags::INVISIBLE != 0),
        "IsCastingAbility" => Value::Boolean(unit.statuses.bits & StatusFlags::CASTING != 0),
        "IsCreep" => Value::Boolean(bota_core::kind_def(unit.kind).creep),
        "IsTower" => Value::Boolean(unit.kind == UnitKind::Tower),
        "GetModifierName" | "GetModifierRemainingDuration" | "GetModifierStackCount" => {
            let index =
                bounded_integer(vm, args, 1, 0, bota_proto::MAX_EFFECTS_PER_UNIT as i32 - 1)?;
            let effect = unit
                .effects
                .get(index as usize)
                .ok_or_else(|| vm.error("modifier index is not present"))?;
            match name {
                "GetModifierName" => {
                    let definition = bota_core::effect_def(effect.id)
                        .ok_or_else(|| vm.error("modifier definition is not known"))?;
                    vm.new_string(definition.key.as_bytes())?
                }
                "GetModifierRemainingDuration" => {
                    Value::Number(effect.ticks_left.map_or(-1.0, seconds))
                }
                _ => Value::Number(f64::from(effect.stacks.unwrap_or(0))),
            }
        }
        _ => return Err(vm.error("unknown modifier query")),
    };
    vm.results(&[value])
}
