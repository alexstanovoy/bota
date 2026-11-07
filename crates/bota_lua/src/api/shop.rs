//! Purchase validation from received inventory, public shops and core recipes.

use bota_client_core::IssuedOrder;
use bota_core::{PurchaseCarrier, Shelf, ShopAccess};
use bota_proto::{Fixed, Order, Team, UnitKind, UnitView};

use super::{context::{ApiContext, register_unit}, globals::string, *};

mod projection;
pub(crate) use projection::Projection;

#[derive(Clone, Copy)]
#[repr(u8)]
enum Purchase { Success = 0, OutOfStock = 1, Disallowed = 2, InsufficientGold = 3, NotAtSecretShop = 6, InvalidName = 7 }

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    for (name, value) in [("PURCHASE_ITEM_SUCCESS",0), ("PURCHASE_ITEM_OUT_OF_STOCK",1), ("PURCHASE_ITEM_DISALLOWED_ITEM",2), ("PURCHASE_ITEM_INSUFFICIENT_GOLD",3), ("PURCHASE_ITEM_NOT_AT_HOME_SHOP",4), ("PURCHASE_ITEM_NOT_AT_SIDE_SHOP",5), ("PURCHASE_ITEM_NOT_AT_SECRET_SHOP",6), ("PURCHASE_ITEM_INVALID_ITEM_NAME",7)] {
        lua.set_global(name, Value::Number(f64::from(value)))?;
    }
    register_unit(lua, context, "ActionImmediate_PurchaseItem", purchase)?;
    register_unit(lua, context, "Action_PurchaseItem", purchase)
}

fn purchase(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let unit = context.receiver(vm, args)?;
    let mut state = context.state.borrow_mut();
    if Some(unit) != state.own_id() { return Err(vm.error("action requires the active bot")); }
    let snapshot = state.snapshot.as_ref().ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(64 + 4 * bota_core::ITEM_COUNT as u64 * bota_core::ITEM_COUNT as u64 + snapshot.units.len() as u64 * (snapshot.players.len() as u64 + 4) + snapshot.stock.len() as u64 * 4 + snapshot.players.len() as u64 * 256)?;
    let key = string(vm, args, 1)?;
    let item = std::str::from_utf8(key).ok().and_then(bota_core::item_by_key);
    let Some(item) = item else { return vm.results(&[Value::Number(f64::from(Purchase::InvalidName as u8))]); };
    let mut projected = state.purchases.clone().unwrap_or_else(|| Projection::observed(snapshot));
    let result = projected.buy(snapshot, unit, item);
    if matches!(result, Purchase::Success) {
        state.order(vm, IssuedOrder::hero(Order::Buy { item }))?;
        state.purchases = Some(projected);
    }
    vm.results(&[Value::Number(f64::from(result as u8))])
}

fn carrier<'a>(map: &bota_core::MapDef, unit: &'a UnitView) -> PurchaseCarrier<'a, bota_proto::ItemView> {
    PurchaseCarrier {
        bag: Shelf { slots: &unit.items, town_portal: (unit.kind == UnitKind::Hero).then_some(&unit.town_portal), combines: true },
        shops: ShopAccess {
            base: unit.team != Team::Neutral && unit.pos.within(bota_core::fountain_pos(map, unit.team), Fixed::from_int(bota_core::rules::SHOP_RANGE)),
            secret: map.secret_shops.iter().any(|shop| bota_core::shop_zone_touches(shop.corners, unit.pos, unit.bound)),
        },
    }
}
