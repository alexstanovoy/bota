//! An explicit baseline equipment policy; purchase rules remain in the shared binding.

use bota_proto::{ItemId, ItemSlot, ItemView, Order, UnitKind};
use luagee::{LuaError, Vm};
use crate::{api::Shared, state::{Snapshot, State}};
use super::context;

const GOALS: &[(&str, u16)] = &[
    ("item_branches", 2), ("item_wraith_band", 1), ("item_bottle", 1),
    ("item_boots", 1), ("item_magic_wand", 1), ("item_power_treads", 1),
    ("item_dragon_lance", 1),
];
const RECIPE_WORK: usize = 1024;

pub(super) fn purchase(vm: &mut Vm, shared: &Shared) -> Result<usize, LuaError> {
    let call = {
        let mut state = shared.borrow_mut();
        let Some((snapshot, own)) = context(&state) else { return Ok(0); };
        if state.orders.iter().any(|order| order.unit.is_none() && matches!(order.order, Order::Buy { .. }))
            || state.player(own.id).and_then(|player| player.gold).is_none_or(|gold| gold <= 0) { return Ok(0); }
        vm.charge_fuel(64 + snapshot.units.len() as u64 * 18)?;
        let Some(item) = goal(vm, &state, snapshot, own.id)? else { return Ok(0); };
        let unit = own.id;
        let name = bota_core::item_def(item).ok_or_else(|| vm.error("equipment goal is not registered"))?.key;
        let table = state.unit_methods.ok_or_else(|| vm.error("purchase binding is unavailable"))?;
        let key = vm.new_string(b"ActionImmediate_PurchaseItem")?;
        let function = vm.raw_get(table, key)?;
        let receiver = state.handle(vm, Some(unit))?;
        let argument = vm.new_string(name.as_bytes())?;
        (function, receiver, argument)
    };
    // Release the observation borrow before the shared purchase callback reserves this turn's resources.
    vm.call(call.0, &[call.1, call.2])?;
    Ok(0)
}

fn goal(vm: &mut Vm, state: &State, snapshot: &Snapshot, unit: bota_proto::EntityId) -> Result<Option<ItemId>, LuaError> {
    let mut held = Vec::with_capacity(24);
    for slot in 0..16 {
        if let Some(item) = state.item(unit, ItemSlot(slot)) && item.owner == state.current_slot { held.push(*item); }
    }
    if let Some(courier) = snapshot.units.iter().find(|known| known.unit.kind == UnitKind::Courier && known.unit.team == state.team && known.unit.owner == Some(state.current_slot)) {
        held.extend(courier.unit.items.iter().take(bota_proto::limits::COURIER_ITEM_SLOTS).flatten().filter(|item| item.owner == state.current_slot).copied());
    }
    let mut work = RECIPE_WORK;
    for &(key, needed) in GOALS {
        let Some(id) = bota_core::item_by_key(key) else { continue; };
        if snapshot.catalogue.sold(id).is_none() { continue; }
        let mut count = 0;
        for item in &held { count += components(vm, *item, id, &mut work)?; }
        if count < needed { return Ok(Some(id)); }
    }
    Ok(None)
}

fn components(vm: &mut Vm, held: ItemView, wanted: ItemId, work: &mut usize) -> Result<u16, LuaError> {
    let mut pending = vec![(held.id, held.mode, 0u8)];
    let mut count = 0u16;
    while let Some((id, mode, depth)) = pending.pop() {
        if *work == 0 || depth > 16 { return Err(vm.error("equipment component projection limit exceeded")); }
        *work -= 1;
        vm.charge_fuel(1)?;
        if id == wanted { count = count.saturating_add(1); continue; }
        let Some(definition) = bota_core::item_def(id) else { continue; };
        let recipe = definition.recipes().find(|recipe| !recipe.by_charges && recipe.mode == mode)
            .or_else(|| definition.recipes().find(|recipe| !recipe.by_charges));
        if let Some(recipe) = recipe {
            if pending.len() + recipe.parts.len() > RECIPE_WORK { return Err(vm.error("equipment component projection limit exceeded")); }
            pending.extend(recipe.parts.iter().map(|part| (*part, None, depth + 1)));
        }
    }
    Ok(count)
}
