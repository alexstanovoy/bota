//! Purchase validation from received inventory, public shops and core recipes.

use bota_client_core::IssuedOrder;
use bota_core::{PurchaseCarrier, Shelf, ShopAccess};
use bota_proto::{Fixed, Order, Team, UnitKind, UnitView};

use super::{
    context::{ApiContext, register_unit},
    globals::string,
    *,
};

mod projection;
pub(crate) use projection::Projection;

/// What a purchase returns to the script: the value of the matching `PURCHASE_ITEM_*` global (the installed game's
/// ids, defined in `constants.rs`; `tests/shop.rs` compares every result with its global).
#[derive(Clone, Copy)]
#[repr(i16)]
enum Purchase {
    Success = -1,
    OutOfStock = 82,
    Disallowed = 78,
    InsufficientGold = 63,
    NotAtSecretShop = 62,
    InvalidName = 33,
}

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_unit(lua, context, "ActionImmediate_PurchaseItem", purchase)?;
    register_unit(lua, context, "Action_PurchaseItem", purchase)?;
    let methods = context
        .state
        .borrow()
        .unit_methods
        .expect("installed unit methods");
    let key = lua.string(b"ActionImmediate_PurchaseItem")?;
    let callback = lua.vm().raw_get(methods, key)?;
    lua.pin(callback)?;
    context.state.borrow_mut().host_purchase = Some(callback);
    Ok(())
}

fn purchase(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let unit = context.receiver(vm, args)?;
    let mut state = context.state.borrow_mut();
    if Some(unit) != state.own_id() {
        return Err(vm.error("action requires the active bot"));
    }
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(
        64 + 4 * bota_core::ITEM_COUNT as u64 * bota_core::ITEM_COUNT as u64
            + snapshot.units.len() as u64 * (snapshot.players.len() as u64 + 4)
            + snapshot.stock.len() as u64 * 4
            + snapshot.players.len() as u64 * 256,
    )?;
    let key = string(vm, args, 1)?;
    let item = std::str::from_utf8(key)
        .ok()
        .and_then(bota_core::item_by_key);
    let Some(item) = item else {
        return vm.results(&[Value::Number(f64::from(Purchase::InvalidName as i16))]);
    };
    let mut projected = state
        .purchases
        .clone()
        .unwrap_or_else(|| Projection::observed(snapshot));
    let result = projected.buy(snapshot, unit, item);
    if matches!(result, Purchase::Success) {
        state.order(vm, IssuedOrder::hero(Order::Buy { item }))?;
        state.purchases = Some(projected);
    }
    vm.results(&[Value::Number(f64::from(result as i16))])
}

fn carrier<'a>(
    map: &bota_core::MapDef,
    unit: &'a UnitView,
) -> PurchaseCarrier<'a, bota_proto::ItemView> {
    PurchaseCarrier {
        bag: Shelf {
            slots: &unit.items,
            town_portal: (unit.kind == UnitKind::Hero).then_some(&unit.town_portal),
            combines: true,
        },
        shops: ShopAccess {
            base: unit.team != Team::Neutral
                && unit.pos.within(
                    bota_core::fountain_pos(map, unit.team),
                    Fixed::from_int(bota_core::rules::SHOP_RANGE),
                ),
            secret: map
                .secret_shops
                .iter()
                .any(|shop| bota_core::shop_zone_touches(shop.corners, unit.pos, unit.bound)),
        },
    }
}
