//! Engine unit-list categories over the receiving side's visible observations.

use super::{
    context::{ApiContext, register_global},
    globals::bounded_integer,
};
use crate::BindingError;
use bota_proto::{Team, UnitKind, UnitView};
use luagee::{Args, Lua, LuaError, Value, Vm};

const CATEGORIES: [(&str, i32); 12] = [
    ("UNIT_LIST_ALL", 0),
    ("UNIT_LIST_ALLIES", 1),
    ("UNIT_LIST_ALLIED_HEROES", 2),
    ("UNIT_LIST_ALLIED_CREEPS", 3),
    ("UNIT_LIST_ALLIED_WARDS", 4),
    ("UNIT_LIST_ALLIED_BUILDINGS", 5),
    ("UNIT_LIST_ENEMIES", 6),
    ("UNIT_LIST_ENEMY_HEROES", 7),
    ("UNIT_LIST_ENEMY_CREEPS", 8),
    ("UNIT_LIST_ENEMY_WARDS", 9),
    ("UNIT_LIST_ENEMY_BUILDINGS", 10),
    ("UNIT_LIST_NEUTRAL_CREEPS", 11),
];

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    for (name, value) in CATEGORIES {
        lua.set_global(name, Value::Number(f64::from(value)))?;
    }
    register_global(lua, context, "GetUnitList", unit_list)
}

fn matches(category: i32, unit: &UnitView, side: Team) -> bool {
    let allied = unit.team == side;
    let enemy = unit.team == side.enemy();
    let definition = bota_core::kind_def(unit.kind);
    match category {
        0 => true,
        1 => allied,
        2 => allied && unit.kind == UnitKind::Hero,
        3 => allied && definition.creep,
        4 => allied && unit.kind == UnitKind::Ward,
        5 => allied && definition.structure,
        6 => enemy,
        7 => enemy && unit.kind == UnitKind::Hero,
        8 => enemy && definition.creep,
        9 => enemy && unit.kind == UnitKind::Ward,
        10 => enemy && definition.structure,
        11 => unit.team == Team::Neutral && definition.creep,
        _ => false,
    }
}

fn unit_list(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let category = bounded_integer(vm, args, 0, 0, 11)?;
    let mut state = context.state.borrow_mut();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(16 + 2 * snapshot.units.len() as u64)?;
    let ids: Vec<_> = snapshot
        .units
        .iter()
        .filter(|known| {
            known.visible && known.unit.hp > 0 && matches(category, &known.unit, state.team)
        })
        .map(|known| known.unit.id)
        .collect();
    let table = vm.new_table()?;
    for (index, id) in ids.into_iter().enumerate() {
        let value = state.handle(vm, Some(id))?;
        vm.raw_set(table, Value::Number((index + 1) as f64), value)?;
    }
    vm.results(&[Value::Table(table)])
}
