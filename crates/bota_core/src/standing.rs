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
            .is_some_and(|def| matches!(def.effect, UseSpec::Teleport(_))),
        Order::Put { .. } | Order::Take { .. } => true,
        _ => false,
    }
}

/// Whether `order` needs the hero standing when it names no unit: all but buying, selling (from
/// the stash), taking an item apart, locking it against combining, the Glyph and a cheat (as
/// Dota's `-respawn`), which a seat whose hero lies dead may still give.
pub const fn order_needs_body(order: Order) -> bool {
    !matches!(
        order,
        Order::Buy { .. }
            | Order::Sell { .. }
            | Order::Disassemble { .. }
            | Order::SetCombineLock { .. }
            | Order::Glyph
            | Order::Cheat { .. }
    )
}

/// Whether `order` waits its turn in its unit's queue when it is given queued: what the body
/// carries out (a move, an attack, a cast, a use, laying an item down or taking one up).
/// Learning, talents, buying, selling, swapping, taking items apart, locking them, cheats and the
/// Glyph are carried out at once,
/// queued or not, and leave the queue as it is.
pub const fn order_queues(order: Order) -> bool {
    match order {
        Order::Move { .. }
        | Order::Attack { .. }
        | Order::Cast { .. }
        | Order::Use { .. }
        | Order::Put { .. }
        | Order::Take { .. }
        | Order::MoveDirection { .. } => true,
        Order::Buy { .. }
        | Order::Sell { .. }
        | Order::Swap { .. }
        | Order::Learn { .. }
        | Order::Cheat { .. }
        | Order::LearnTalent { .. }
        | Order::LearnAttributeBonus
        | Order::SwitchWard { .. }
        | Order::Disassemble { .. }
        | Order::SetCombineLock { .. }
        | Order::Glyph => false,
    }
}
