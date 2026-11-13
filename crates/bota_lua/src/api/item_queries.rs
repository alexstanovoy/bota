//! Named item lookup in received storage, with the existing inventory slot layout.

use super::{
    context::{ApiContext, register_unit},
    globals::string,
};
use crate::BindingError;
use bota_proto::ItemSlot;
use luagee::{Args, Lua, LuaError, Value, Vm};

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_unit(lua, context, "FindItemSlot", find_item_slot)
}

fn find_item_slot(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let id = context.receiver(vm, args)?;
    if !matches!(vm.arg(args, 1), Value::String(_)) {
        return Err(vm.arg_error(args, 1, "item name expected"));
    }
    let state = context.state.borrow();
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    let slots = u64::from(ItemSlot::TOWN_PORTAL.0) + 1;
    vm.charge_fuel(16 + slots * (snapshot.units.len() as u64 + snapshot.players.len() as u64 + 8))?;
    let observed = state.unit(id).is_some()
        || state.player(id).is_some_and(|player| {
            player.team == state.team && (player.kit.is_some() || player.stash.is_some())
        });
    if !observed {
        return Err(vm.error("unit inventory is not observed"));
    }
    let key = string(vm, args, 1)?;
    let found = (0..=ItemSlot::TOWN_PORTAL.0).find(|&slot| {
        state
            .item(id, ItemSlot(slot))
            .and_then(|item| bota_core::item_def(item.id))
            .is_some_and(|definition| definition.key.as_bytes() == key)
    });
    vm.results(&[Value::Number(found.map_or(-1.0, f64::from))])
}
