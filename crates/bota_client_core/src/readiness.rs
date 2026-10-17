//! Waits the wire does not report, known from the orders given.

use bota_core::item_def;
use bota_proto::{ItemId, Order};

use crate::latency::OrderLatency;
use crate::orders::{ControlledUnit, IssuedOrder, item_in};
use crate::session::ClientSession;
use crate::timer_journal::TimerJournal;

/// The wait in ticks a use of `item` puts on every item of its kind the body
/// carries, as the item table says (the Town Portal Scroll's); absent for an
/// item whose wait is its own, which the wire reports.
pub fn shared_wait(item: ItemId) -> Option<u32> {
    item_def(item)
        .filter(|def| def.shared_wait)
        .map(|def| def.cooldown)
}
/// Waits shared by all of a body's copies of an item, which the wire does not
/// report: kept from the orders the session gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SharedWaits {
    journals: [TimerJournal<u32>; 2],
}

impl Default for SharedWaits {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedWaits {
    pub const fn new() -> Self {
        Self {
            journals: [TimerJournal::new(); 2],
        }
    }

    /// Starts the wait the order with `sequence`, given on the snapshot of
    /// `given_tick`, put on `body`.
    pub(crate) fn start(&mut self, body: ControlledUnit, sequence: u32, given_tick: u32) {
        self.journals[body.index()].push(sequence, given_tick);
    }

    /// Removes every wait the rejected order started.
    pub fn reject(&mut self, sequence: u32) -> bool {
        let mut removed = false;
        for journal in &mut self.journals {
            removed |= journal.reject(sequence);
        }
        removed
    }

    /// Ticks a body still owes before it may use an item again, at tick
    /// `now`, planning with `latency`; absent for items without a shared wait.
    pub fn wait_left(
        &self,
        body: ControlledUnit,
        item: ItemId,
        now: u32,
        latency: OrderLatency,
    ) -> Option<u32> {
        let wait = shared_wait(item)?;
        Some(
            self.journals[body.index()]
                .current()
                .map_or(0, |given_tick| {
                    let ready = given_tick
                        .saturating_add(latency.lands())
                        .saturating_add(wait);
                    ready.saturating_sub(now)
                }),
        )
    }
}

/// The body whose shared wait an order starts, if it uses an item that has one.
pub(crate) fn wait_started_by(
    session: &ClientSession,
    issued: IssuedOrder,
) -> Option<ControlledUnit> {
    let Order::Use { slot, .. } = issued.order else {
        return None;
    };
    let body = ControlledUnit::of(issued.unit);
    let unit = match body {
        ControlledUnit::Hero => session.own_hero()?,
        ControlledUnit::Courier => session.own_courier()?,
    };
    shared_wait(item_in(unit, slot)?.id).map(|_| body)
}
