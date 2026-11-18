//! Heroes a player orders that are not its own. In a match with cheats on, every hero a cheat
//! created is controllable by every player, as Dota's Hero Demo makes each hero it spawns
//! (`SetControllableByAllPlayers( true )`, `hero_demo/scripts/vscripts/events.lua`): a player
//! selects it and orders it as its own. An order to it is the order its own seat would give: the
//! hero's gold buys for it, its points learn for it. What belongs to a seat or a side (the
//! Glyph, the Scan, a buyback, a cheat) stays the player's own, whatever it names.

use bota_proto::{EntityId, Order, SlotId};

use crate::game::{World, wire_id};

impl World {
    /// The seat of a hero a cheat created, or of a unit of that seat, when the seat at `slot` may
    /// order it through `named`: cheats are on, `slot` is a player's seat, and `named` is the
    /// created seat's, dead or alive.
    pub fn controlled_seat(&self, slot: SlotId, named: EntityId) -> Option<SlotId> {
        if !self.cheats || self.seat(slot)?.created {
            return None;
        }
        let owner = self
            .seats
            .iter()
            .find(|seat| seat.unit.map(wire_id) == Some(named))
            .map(|seat| seat.slot)
            .or_else(|| {
                self.of_wire(named)
                    .and_then(|unit| self.owner.get(unit).copied())
            })?;
        self.seat(owner)
            .filter(|seat| seat.created && seat.slot != slot)
            .map(|seat| seat.slot)
    }

    /// The seat an order is carried out as: the hero's own for what a hero the player controls
    /// does ([`Reach::Unit`]), the giver's for everything else.
    pub fn acting_seat(&self, slot: SlotId, named: Option<EntityId>, order: &Order) -> SlotId {
        if order_reach(order) == Reach::Seat {
            return slot;
        }
        named
            .and_then(|named| self.controlled_seat(slot, named))
            .unwrap_or(slot)
    }
}

/// Whose an order is, for a player who controls heroes not its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// What a hero does with its body, its book, its bag and its shop: carried out as the seat
    /// of the hero it names.
    Unit,
    /// What belongs to a seat or a side (its Glyph, its Scan, its buyback, its cheats): always
    /// the giver's own, whatever unit it names.
    Seat,
}

/// Every order's reach. No arm is a catch-all, so an order added to the protocol must say
/// whose it is before it compiles.
pub const fn order_reach(order: &Order) -> Reach {
    match order {
        Order::Move { .. }
        | Order::Attack { .. }
        | Order::Cast { .. }
        | Order::Use { .. }
        | Order::Put { .. }
        | Order::Take { .. }
        | Order::Buy { .. }
        | Order::Sell { .. }
        | Order::Swap { .. }
        | Order::Learn { .. }
        | Order::LearnTalent { .. }
        | Order::LearnAttributeBonus
        | Order::SwitchWard { .. }
        | Order::MoveDirection { .. }
        | Order::Disassemble { .. }
        | Order::SetCombineLock { .. } => Reach::Unit,
        Order::Cheat { .. } | Order::Glyph | Order::Scan { .. } | Order::Buyback => Reach::Seat,
    }
}
