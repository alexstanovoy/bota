//! Waits the wire does not report, known from the orders given.

use bota_core::item_def;
use bota_proto::{ItemId, Order};

use crate::latency::OrderLatency;
use crate::orders::{ControlledUnit, IssuedOrder, item_in};
use crate::session::ClientSession;

/// The wait in ticks a use of `item` puts on every item of its kind the body
/// carries, as the item table says (the Town Portal Scroll's); absent for an
/// item whose wait is its own, which the wire reports.
pub fn shared_wait(item: ItemId) -> Option<u32> {
    item_def(item)
        .filter(|def| def.shared_wait)
        .map(|def| def.cooldown)
}
/// Recent timers kept per journal so a rejected order's timers can be rolled
/// back. An evicted timer becomes the base and can no longer be rejected; with
/// one order outstanding at a time that is never needed.
pub const MAX_TIMER_HISTORY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timer {
    sequence: u32,
    /// The tick of the snapshot the order was decided on.
    given_tick: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimerJournal {
    base: Option<Timer>,
    entries: [Option<Timer>; MAX_TIMER_HISTORY],
    count: usize,
}

impl TimerJournal {
    const fn new() -> Self {
        Self {
            base: None,
            entries: [None; MAX_TIMER_HISTORY],
            count: 0,
        }
    }

    fn push(&mut self, timer: Timer) {
        if self.count == MAX_TIMER_HISTORY {
            self.base = self.entries[0];
            self.entries.copy_within(1..MAX_TIMER_HISTORY, 0);
            self.count -= 1;
        }
        self.entries[self.count] = Some(timer);
        self.count += 1;
    }

    fn reject(&mut self, sequence: u32) -> bool {
        let mut kept = [None; MAX_TIMER_HISTORY];
        let mut count = 0usize;
        let mut removed = false;
        for timer in self.entries[..self.count].iter().flatten().copied() {
            if timer.sequence == sequence {
                removed = true;
            } else {
                kept[count] = Some(timer);
                count += 1;
            }
        }
        self.entries = kept;
        self.count = count;
        removed
    }

    fn current(&self) -> Option<Timer> {
        self.entries[..self.count]
            .iter()
            .rev()
            .find_map(|timer| *timer)
            .or(self.base)
    }
}

/// Waits shared by all of a body's copies of an item, which the wire does not
/// report: kept from the orders the session gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SharedWaits {
    journals: [TimerJournal; 2],
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
        self.journals[body.index()].push(Timer {
            sequence,
            given_tick,
        });
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
        Some(self.journals[body.index()].current().map_or(0, |timer| {
            let ready = timer
                .given_tick
                .saturating_add(latency.lands())
                .saturating_add(wait);
            ready.saturating_sub(now)
        }))
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
