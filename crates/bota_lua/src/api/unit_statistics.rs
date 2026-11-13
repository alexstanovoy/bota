//! Numeric unit reads in engine units, directly from received observations.

use super::{context::ApiContext, globals::bounded_integer, *};
use bota_proto::Attribute;

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    for (name, value) in [
        ("ATTRIBUTE_STRENGTH", 0),
        ("ATTRIBUTE_AGILITY", 1),
        ("ATTRIBUTE_INTELLECT", 2),
        ("ATTRIBUTE_INVALID", -1),
    ] {
        lua.set_global(name, Value::Number(f64::from(value)))?;
    }
    for name in [
        "GetMana",
        "GetMaxMana",
        "GetHealthRegen",
        "GetManaRegen",
        "GetArmor",
        "GetAttackDamage",
        "GetAttackRange",
        "GetSecondsPerAttack",
        "GetAttackPoint",
        "GetAttackSpeed",
        "GetCurrentMovementSpeed",
        "GetBoundingRadius",
        "GetEvasion",
        "GetFacing",
        "GetPrimaryAttribute",
        "GetAttributeValue",
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

#[expect(
    clippy::float_arithmetic,
    reason = "received integer statistics are converted to the API's fixed units"
)]
fn scaled(value: f64, scale: f64) -> f64 {
    value / scale
}

fn fixed(value: bota_proto::Fixed) -> f64 {
    scaled(f64::from(value.raw), f64::from(bota_proto::Fixed::ONE.raw))
}

fn attribute(value: Attribute) -> i32 {
    match value {
        Attribute::Strength => 0,
        Attribute::Agility => 1,
        Attribute::Intelligence => 2,
    }
}

fn read(context: &ApiContext, vm: &mut Vm, args: Args, name: &str) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    let chosen = if name == "GetAttributeValue" {
        Some(bounded_integer(vm, args, 1, 0, 2)?)
    } else {
        None
    };
    let state = context.state.borrow();
    vm.charge_fuel(
        16 + state
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.units.len()) as u64
            / 8,
    )?;
    let unit = state
        .unit(id)
        .map(crate::state::UnitStatistics::from)
        .or_else(|| state.player(id).and_then(|player| player.statistics))
        .ok_or_else(|| vm.error("unit is no longer known"))?;
    let value = match name {
        "GetMana" => f64::from(unit.mana),
        "GetMaxMana" => f64::from(unit.max_mana),
        "GetHealthRegen" => fixed(unit.hp_regen),
        "GetManaRegen" => fixed(unit.mana_regen),
        "GetArmor" => fixed(unit.armor),
        "GetAttackDamage" => f64::from(unit.attack_damage),
        "GetAttackRange" => f64::from(unit.attack_range.to_int()),
        "GetSecondsPerAttack" => scaled(f64::from(unit.attack_time), 1000.0),
        "GetAttackPoint" => scaled(
            f64::from(
                super::unit_data::base_definition(&state, id)
                    .ok_or_else(|| vm.error("unit base attack data is not known"))?
                    .attack_point,
            ),
            1000.0,
        ),
        "GetAttackSpeed" => scaled(f64::from(unit.attack_speed), 100.0),
        "GetCurrentMovementSpeed" => f64::from(unit.move_speed.to_int()),
        "GetBoundingRadius" => fixed(unit.bound),
        "GetEvasion" => fixed(unit.evasion),
        "GetFacing" => f64::from(i32::from(unit.facing.brads) * 360 / 65536),
        "GetPrimaryAttribute" => f64::from(unit.primary.map_or(-1, attribute)),
        "GetAttributeValue" => f64::from(
            unit.attributes
                .of(match chosen {
                    Some(0) => Attribute::Strength,
                    Some(1) => Attribute::Agility,
                    Some(2) => Attribute::Intelligence,
                    _ => return Err(vm.error("attribute index is outside its bounds")),
                })
                .to_int(),
        ),
        _ => return Err(vm.error("unknown unit statistic")),
    };
    vm.results(&[Value::Number(value)])
}
