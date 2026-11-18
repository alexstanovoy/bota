//! Heroes a player orders that are not its own. In a match with cheats on, every hero a cheat
//! created is controllable by every player, as Dota's Hero Demo makes each hero it spawns
//! (`SetControllableByAllPlayers( true )`, `hero_demo/scripts/vscripts/events.lua`): a player
//! selects it and orders it as its own. An order to it is the order its own seat would give: the
//! hero's gold buys for it, its points learn for it. A cheat stays the player's, aimed at it.

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

    /// The seat an order is carried out as: the hero's own for an order a player gives a hero it
    /// controls, the giver's for everything else. A cheat is the giver's wherever it is aimed.
    pub fn acting_seat(&self, slot: SlotId, named: Option<EntityId>, order: &Order) -> SlotId {
        if matches!(order, Order::Cheat { .. }) {
            return slot;
        }
        named
            .and_then(|named| self.controlled_seat(slot, named))
            .unwrap_or(slot)
    }
}
