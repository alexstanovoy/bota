//! What an order does to the move or attack its unit stands on, so a client's order ledger and
//! the server keep one rule.

use bota_proto::{AbilityId, Aim, ItemId, Order};

use crate::{UseSpec, ability_def, item_def};

/// Whether `order` ends the standing move or attack of the unit it is for, given the ability or
/// item in the slot it names. A move or an attack replaces the standing order itself and is not
/// asked about. A cast aimed anywhere but at its caster takes the body over; putting an item
/// down or picking one up leaves the unit standing once done; a scroll's teleport leaves it idle
/// where it lands. A cast at oneself, any other item use and the orders that do not move the
/// body (learning, buying, swapping, ...) leave it walking on.
#[inline]
pub fn ends_standing_order(order: Order, ability: Option<AbilityId>, item: Option<ItemId>) -> bool {
    match order {
        Order::Cast { .. } => ability
            .and_then(ability_def)
            .is_some_and(|def| def.aim != Aim::Own),
        Order::Use { .. } => item
            .and_then(item_def)
            .is_some_and(|def| matches!(def.effect, UseSpec::Teleport)),
        Order::Put { .. } | Order::Take { .. } => true,
        _ => false,
    }
}
