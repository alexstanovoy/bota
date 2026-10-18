//! Waits the wire does not report, known from the orders given.

use bota_core::{item_def, rules};
use bota_proto::{ItemId, ItemSlot, Order};

use crate::latency::OrderLatency;
use crate::limits::HERO_ITEM_SLOTS;
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
/// Waits the wire does not report, kept from the orders the session gave: the
/// wait all of a body's copies of an item share, and the mute a swap with the
/// backpack puts on an inventory slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalWaits {
    journals: [TimerJournal<u32>; 2],
    mutes: [TimerJournal<u32>; rules::INVENTORY_SLOTS],
}

impl LocalWaits {
    /// Starts the wait the order with `sequence`, given on the snapshot of
    /// `given_tick`, put on `body`.
    pub(crate) fn start(&mut self, body: ControlledUnit, sequence: u32, given_tick: u32) {
        self.journals[body.index()].push(sequence, given_tick);
    }

    /// Starts the mutes the swap `issued` with `sequence` puts on the hero's
    /// inventory slots, from the tick it takes effect, `latency` after the
    /// snapshot of `given_tick`. A courier's bag has no backpack and stash
    /// moves never mute.
    pub(crate) fn start_mutes(
        &mut self,
        sequence: u32,
        issued: IssuedOrder,
        given_tick: u32,
        latency: OrderLatency,
    ) {
        let (Order::Swap { from, to }, None) = (issued.order, issued.unit) else {
            return;
        };
        let apply_tick = given_tick.saturating_add(latency.lands());
        let from = usize::from(from.0);
        let to = usize::from(to.0);
        let ready_tick = apply_tick.saturating_add(rules::BACKPACK_MUTE_TICKS);
        if (rules::INVENTORY_SLOTS..HERO_ITEM_SLOTS).contains(&from) && to < rules::INVENTORY_SLOTS
        {
            self.mutes[to].push(sequence, ready_tick);
            return;
        }
        if (rules::INVENTORY_SLOTS..HERO_ITEM_SLOTS).contains(&to) && from < rules::INVENTORY_SLOTS
        {
            self.mutes[from].push(sequence, ready_tick);
            return;
        }
        if from < rules::INVENTORY_SLOTS && to < rules::INVENTORY_SLOTS {
            self.follow_active_mute(sequence, from, to, apply_tick);
        }
    }

    /// A swap inside the inventory carries each slot's running mute to the other.
    fn follow_active_mute(&mut self, sequence: u32, from: usize, to: usize, apply_tick: u32) {
        let running = |journal: &TimerJournal<u32>| {
            journal
                .current()
                .filter(|ready_tick| *ready_tick > apply_tick)
        };
        let (from_ready, to_ready) = (running(&self.mutes[from]), running(&self.mutes[to]));
        if let Some(ready_tick) = from_ready {
            self.mutes[to].push(sequence, ready_tick);
        }
        if let Some(ready_tick) = to_ready {
            self.mutes[from].push(sequence, ready_tick);
        }
    }

    /// Removes every wait the rejected order started.
    pub fn reject(&mut self, sequence: u32) -> bool {
        let mut removed = false;
        for journal in self.journals.iter_mut().chain(&mut self.mutes) {
            removed |= journal.reject(sequence);
        }
        removed
    }

    /// Ticks the hero's inventory `slot` stays muted by a swap, at tick `now`;
    /// absent for a courier and for slots outside the inventory.
    pub fn mute_left(&self, body: ControlledUnit, slot: ItemSlot, now: u32) -> Option<u32> {
        if body != ControlledUnit::Hero {
            return None;
        }
        let journal = self.mutes.get(usize::from(slot.0))?;
        Some(
            journal
                .current()
                .map_or(0, |ready| ready.saturating_sub(now)),
        )
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
