//! Names and numeric conventions over public catalogue and clock queries.
use super::{
    context::{ApiContext, register_global, register_unit},
    globals::{integer, string},
    *,
};
use bota_core::StorageClass;

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_global(lua, context, "IsLocationVisible", visible)?;
    register_global(lua, context, "GetHeightLevel", height)?;
    register_global(lua, context, "GetTimeOfDay", time_of_day)?;
    register_unit(lua, context, "GetItemSlotType", slot_type)?;
    register_global(lua, context, "GetItemStockCount", stock)
}
fn height(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    vm.charge_fuel(8)?;
    let position = vector::position(vm, vm.arg(args, 0))?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.results(&[Value::Number(f64::from(
        snapshot.statics.ground_tier(position),
    ))])
}
#[expect(
    clippy::float_arithmetic,
    reason = "the API's clock is a normalized fraction"
)]
fn time_of_day(context: &ApiContext, vm: &mut Vm, _: Args) -> Result<usize, LuaError> {
    vm.charge_fuel(8)?;
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    let (phase, cycle) = bota_core::day_phase(
        snapshot.day_tick,
        bota_core::rules::Schedule::of(snapshot.horn),
    );
    vm.results(&[Value::Number(f64::from((phase as f32) / (cycle as f32)))])
}
fn slot_type(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    vm.charge_fuel(8)?;
    let id = context.receiver(vm, args)?;
    let state = context.state.borrow();
    vm.charge_fuel(
        state
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.units.len()) as u64,
    )?;
    let inventory =
        state.unit(id).is_some_and(|unit| !unit.items.is_empty()) || state.player(id).is_some();
    let slot = integer(vm, args, 1)?;
    let value = i32::try_from(slot)
        .ok()
        .and_then(bota_core::storage_class)
        .map_or(-1, |class| match class {
            StorageClass::Inventory => 0,
            StorageClass::Backpack => 1,
            StorageClass::Stash => 2,
            StorageClass::Dedicated => -1,
        });
    vm.results(&[Value::Number(f64::from(if inventory { value } else { -1 }))])
}
fn stock(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let length = vm.bytes(vm.arg(args, 0)).map_or(0, |key| key.len());
    vm.charge_fuel(8 + length as u64 + bota_core::ITEMS.len() as u64)?;
    let key = string(vm, args, 0)?;
    let item = std::str::from_utf8(key)
        .ok()
        .and_then(bota_core::item_by_key);
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(snapshot.stock.len() as u64)?;
    let value = item
        .and_then(|item| bota_client_core::item_stock(&snapshot.stock, state.team, item))
        .map_or(Value::Nil, |count| Value::Number(f64::from(count)));
    vm.results(&[value])
}

fn visible(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let at = vector::position(vm, vm.arg(args, 0))?;
    let mut state = context.state.borrow_mut();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    let build = state
        .sight
        .as_ref()
        .is_none_or(|ground| !ground.fits_forest(&snapshot.felled_trees, &snapshot.planted_trees));
    let grid = bota_core::rules::GRID_CELLS as u64;
    vm.charge_fuel(
        16 + snapshot.felled_trees.len() as u64
            + snapshot.planted_trees.len() as u64
            + (snapshot.units.len() + snapshot.map.base_blockers.len()) as u64 * (grid + 1),
    )?;
    if build {
        vm.charge_fuel(
            3 * grid * grid
                + snapshot.statics.opaque_cells().len() as u64
                + snapshot.statics.trees().len() as u64 * 16
                + snapshot.planted_trees.len() as u64 * 16,
        )?;
        let ground = bota_client_core::SightGround::lay_forest(
            &snapshot.statics,
            &snapshot.felled_trees,
            &snapshot.planted_trees,
        )
        .ok_or_else(|| vm.error("visibility grid is unavailable"))?;
        state.sight = Some(ground);
    }
    let snapshot = state.snapshot.as_ref().expect("current view");
    let result = state.sight.as_ref().expect("forest laid").sees_units(
        snapshot
            .units
            .iter()
            .filter(|known| known.visible)
            .map(|known| &known.unit),
        state.team,
        at,
    );
    vm.results(&[Value::Boolean(result)])
}
