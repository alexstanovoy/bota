//! Buyback queries and the buyback action, from what the bot's own side is told.

use bota_client_core::IssuedOrder;
use bota_proto::{BuybackView, EntityId, Order};

use super::{
    CALL_FUEL,
    context::{ApiContext, register_unit},
    globals::seconds,
    *,
};
use crate::state::State;

pub(super) fn install(lua: &mut Lua, context: &ApiContext) -> Result<(), BindingError> {
    register_unit(lua, context, "GetBuybackCooldown", cooldown)?;
    register_unit(lua, context, "GetBuybackCost", cost)?;
    register_unit(lua, context, "HasBuyback", has)?;
    register_unit(lua, context, "ActionImmediate_Buyback", buy_back)
}

/// What a teammate hero's seat tells of its buyback, with the gold it holds after this turn's
/// purchases and whether it may buy back by everything told. Nothing for any other unit: the
/// other side's buyback is never received.
struct Told {
    buyback: BuybackView,
    gold: i32,
    tick: u32,
    may: bool,
}

fn told(state: &State, vm: &mut Vm, unit: EntityId) -> Result<Option<Told>, LuaError> {
    let snapshot = state
        .snapshot
        .as_ref()
        .ok_or_else(|| vm.error("a current view is required"))?;
    vm.charge_fuel(CALL_FUEL + snapshot.players.len() as u64)?;
    let Some(player) = snapshot
        .players
        .iter()
        .find(|player| player.unit == Some(unit) && player.team == snapshot.team)
    else {
        return Ok(None);
    };
    let (Some(buyback), Some(held)) = (player.buyback, player.gold) else {
        return Ok(None);
    };
    let gold = state
        .purchases
        .as_ref()
        .and_then(|projection| projection.gold(player.slot))
        .unwrap_or(held);
    let reincarnating = player.kit.as_ref().is_some_and(|kit| kit.reincarnating);
    let may = bota_core::buyback_refusal(
        player.respawn_left > 0,
        gold,
        buyback.cost,
        snapshot.tick,
        buyback.ready_at,
        reincarnating,
    )
    .is_ok();
    Ok(Some(Told {
        buyback,
        gold,
        tick: snapshot.tick,
        may,
    }))
}

/// The game time buyback is available from, as the engine's own getter answers: a time, not
/// the seconds left, and -1 for a unit that is no teammate hero.
fn cooldown(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let unit = context.receiver(vm, args)?;
    let told = told(&context.state.borrow(), vm, unit)?;
    vm.results(&[Value::Number(
        told.map_or(-1.0, |told| seconds(told.buyback.ready_at)),
    )])
}

fn cost(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let unit = context.receiver(vm, args)?;
    let told = told(&context.state.borrow(), vm, unit)?;
    vm.results(&[Value::Number(
        told.map_or(-1.0, |told| f64::from(told.buyback.cost)),
    )])
}

/// The engine's own test: the cooldown is strictly past and the gold strictly above the cost.
/// It asks neither whether the hero lies dead nor whether it is reincarnating.
fn has(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let unit = context.receiver(vm, args)?;
    let told = told(&context.state.borrow(), vm, unit)?;
    vm.results(&[Value::Boolean(told.is_some_and(|told| {
        told.buyback.ready_at < told.tick && told.gold > told.buyback.cost
    }))])
}

/// Buys the active bot's dead hero back when everything told allows it, and does nothing
/// otherwise, as the engine's buyback checks silently.
fn buy_back(context: &ApiContext, vm: &mut Vm, args: Args) -> Result<usize, LuaError> {
    let unit = context.receiver(vm, args)?;
    let mut state = context.state.borrow_mut();
    if Some(unit) != state.own_id() {
        return Err(vm.error("action requires the active bot"));
    }
    let asked = state
        .orders
        .iter()
        .any(|issued| issued.order == Order::Buyback);
    if told(&state, vm, unit)?.is_some_and(|told| told.may) && !asked {
        state.order(vm, IssuedOrder::hero(Order::Buyback))?;
    }
    Ok(0)
}
