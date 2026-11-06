//! Predicted item waits between an order and its authoritative snapshot.

use bota_core::{item_def, rules};
use bota_proto::{ItemId, ItemSlot, ItemView, Order};

use crate::latency::OrderLatency;
use crate::orders::{ControlledUnit, IssuedOrder, item_in};
use crate::session::ClientSession;
use crate::timer_journal::TimerJournal;

const SWAP_SLOTS: usize = rules::TOWN_PORTAL_SLOT;

pub(crate) struct BackpackContext {
    slots: [SwapItem; SWAP_SLOTS],
    at_shop: bool,
    confirmed_sequence: u32,
}

pub(crate) fn backpack_context(session: &ClientSession) -> BackpackContext {
    let hero = session.own_hero();
    let player = session.own_player();
    let bag = hero
        .map(|hero| &hero.items)
        .or_else(|| player.and_then(|p| p.kit.as_ref().map(|k| &k.items)));
    let tick = session.current().map_or(0, |view| view.tick);
    BackpackContext {
        slots: std::array::from_fn(|slot| {
            let item = if slot < crate::limits::HERO_ITEM_SLOTS {
                bag.and_then(|b| b.get(slot)).copied().flatten()
            } else {
                player
                    .and_then(|p| p.stash.as_ref())
                    .and_then(|s| s.get(slot - crate::limits::HERO_ITEM_SLOTS))
                    .copied()
                    .flatten()
            };
            SwapItem::snapshot(item, tick, slot, player.and_then(|p| p.backpack))
        }),
        confirmed_sequence: session.confirmed_sequence(),
        at_shop: hero
            .is_some_and(|hero| bota_core::at_any_shop(session.map(), hero.pos, hero.bound)),
    }
}

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
    swaps: [TimerJournal<SwapItem>; SWAP_SLOTS],
    revision: u64,
}

/// Current wait facts and their mutation identity, without rollback history.
/// Suitable for stored observations and provenance; it cannot issue or reject orders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalWaitsSnapshot {
    shared_given: [Option<u32>; 2],
    muted_until: [u32; rules::INVENTORY_SLOTS],
    revision: u64,
}

const _: () = assert!(std::mem::size_of::<LocalWaitsSnapshot>() <= 64);

impl LocalWaitsSnapshot {
    pub fn mute_left(&self, body: ControlledUnit, slot: ItemSlot, now: u32) -> Option<u32> {
        if body != ControlledUnit::Hero {
            return None;
        }
        self.muted_until
            .get(usize::from(slot.0))
            .map(|ready| ready.saturating_sub(now))
    }

    pub fn wait_left(
        &self,
        body: ControlledUnit,
        item: ItemId,
        now: u32,
        latency: OrderLatency,
    ) -> Option<u32> {
        shared_left(self.shared_given[body.index()], item, now, latency)
    }
}

fn shared_left(given: Option<u32>, item: ItemId, now: u32, latency: OrderLatency) -> Option<u32> {
    let wait = shared_wait(item)?;
    Some(given.map_or(0, |tick| {
        tick.saturating_add(latency.lands())
            .saturating_add(wait)
            .saturating_sub(now)
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SwapItem {
    item: Option<ItemView>,
    applies: u32,
    muted_until: u32,
    grace_until: u32,
    used: bool,
}

impl SwapItem {
    fn snapshot(
        item: Option<ItemView>,
        tick: u32,
        slot: usize,
        backpack: Option<bota_proto::BackpackView>,
    ) -> Self {
        let grace = backpack
            .and_then(|b| {
                slot.checked_sub(rules::INVENTORY_SLOTS)
                    .and_then(|slot| b.grace_left.get(slot).copied())
            })
            .unwrap_or(0);
        Self {
            item,
            applies: tick,
            muted_until: tick.saturating_add(item.map_or(0, |i| i.mute_left)),
            grace_until: tick.saturating_add(u32::from(grace)),
            used: backpack.is_none_or(|b| b.used & (1 << slot) != 0),
        }
    }

    fn land(&mut self, from: usize, to: usize, applies: u32, at_shop: bool) {
        self.applies = applies;
        if self.item.is_none() {
            self.muted_until = applies;
            return;
        }
        if at_shop {
            self.muted_until = applies;
        }
        let backpack = |slot| {
            (rules::INVENTORY_SLOTS..rules::INVENTORY_SLOTS + rules::BACKPACK_SLOTS).contains(&slot)
        };
        let was_backpack = backpack(from);
        let is_backpack = backpack(to);
        if !was_backpack && is_backpack {
            self.grace_until = if self.used {
                0
            } else {
                applies.saturating_add(rules::BACKPACK_GRACE_TICKS)
            };
            self.used = true;
        } else if was_backpack && !is_backpack {
            self.muted_until = if at_shop {
                applies
            } else if to >= rules::INVENTORY_SLOTS || self.grace_until > applies {
                self.muted_until
            } else {
                self.muted_until
                    .max(applies.saturating_add(rules::BACKPACK_MUTE_TICKS))
            };
            self.grace_until = 0;
        }
    }
    fn merge_from(
        &mut self,
        source: &mut Self,
        from: usize,
        to: usize,
        applies: u32,
        at_shop: bool,
    ) -> bool {
        let (Some(mut target), Some(mut incoming)) = (self.item, source.item) else {
            return false;
        };
        if !crate::item_merge::merge_item_views(
            &mut target,
            &mut incoming,
            to < rules::INVENTORY_SLOTS + rules::BACKPACK_SLOTS,
        ) {
            return false;
        }
        if self.item.is_some_and(|item| item.id != target.id) && source.used {
            self.grace_until = self.grace_until.min(source.grace_until);
        }
        self.muted_until = self.muted_until.max(source.muted_until);
        if (rules::INVENTORY_SLOTS..rules::INVENTORY_SLOTS + rules::BACKPACK_SLOTS).contains(&from)
            && to < rules::INVENTORY_SLOTS
            && source.grace_until <= applies
        {
            self.muted_until = self
                .muted_until
                .max(applies.saturating_add(rules::BACKPACK_MUTE_TICKS));
        }
        if at_shop {
            self.muted_until = applies;
            source.muted_until = applies;
        }
        self.used |= source.used;
        self.item = Some(target);
        source.item = (bota_core::HeldItem::charges(&incoming) > 0
            || bota_core::HeldItem::sentries(&incoming) > 0)
            .then_some(incoming);
        if source.item.is_none() {
            source.grace_until = 0;
        }
        self.applies = applies;
        source.applies = applies;
        true
    }
}

impl LocalWaits {
    /// Capture only facts consumed by readiness and feature provenance.
    pub fn snapshot(&self) -> LocalWaitsSnapshot {
        LocalWaitsSnapshot {
            shared_given: std::array::from_fn(|body| self.journals[body].current()),
            muted_until: std::array::from_fn(|slot| {
                self.swaps[slot]
                    .current()
                    .map_or(0, |item| item.muted_until)
            }),
            revision: self.revision,
        }
    }

    fn changed(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("validated u32 ticks and issued sequences bound wait revisions below u64");
    }

    /// Starts the wait the order with `sequence`, given on the snapshot of
    /// `given_tick`, put on `body`.
    pub(crate) fn start(&mut self, body: ControlledUnit, sequence: u32, given_tick: u32) {
        self.journals[body.index()].push(sequence, given_tick);
        self.changed();
    }

    /// Starts the mutes the swap `issued` with `sequence` puts on the hero's
    /// inventory slots, from the tick it takes effect, `latency` after the
    /// snapshot of `given_tick`. A courier's bag has no backpack and stash
    /// moves never mute. A shop landing clears inactivity, including
    /// any earlier timer on the inventory slot.
    pub(crate) fn start_mutes(
        &mut self,
        sequence: u32,
        issued: IssuedOrder,
        given_tick: u32,
        latency: OrderLatency,
        context: &BackpackContext,
    ) {
        let (Order::Swap { from, to }, None) = (issued.order, issued.unit) else {
            return;
        };
        let apply_tick = given_tick.saturating_add(latency.lands());
        let from = usize::from(from.0);
        let to = usize::from(to.0);
        if from >= SWAP_SLOTS || to >= SWAP_SLOTS {
            return;
        }
        let predicted = |slot: usize| {
            self.swaps[slot]
                .current_entry()
                .filter(|(sequence, _)| *sequence > context.confirmed_sequence)
                .map(|(_, item)| item)
                .unwrap_or_else(|| context.slots[slot])
        };
        let mut moved = predicted(from);
        let mut displaced = predicted(to);
        let merged = displaced.merge_from(&mut moved, from, to, apply_tick, context.at_shop);
        let landed = if merged {
            [(from, moved), (to, displaced)]
        } else {
            moved.land(from, to, apply_tick, context.at_shop);
            displaced.land(to, from, apply_tick, context.at_shop);
            [(to, moved), (from, displaced)]
        };
        for (slot, item) in landed {
            self.swaps[slot].push(sequence, item);
        }
        self.changed();
    }

    pub(crate) fn clear_swap_predictions(&mut self) {
        if self.swaps.iter().any(|journal| journal.current().is_some()) {
            self.swaps = Default::default();
            self.changed();
        }
    }

    /// Predictions cover orders not yet reflected in the snapshot. Afterwards
    /// the actual stack owns its clock: a new item cannot inherit an old slot's
    /// timer. Shop entry clears inactivity before the normal deadline too.
    pub(crate) fn synchronize(&mut self, tick: u32, context: &BackpackContext) {
        for (slot, journal) in self.swaps.iter_mut().enumerate() {
            if journal
                .current_entry()
                .is_some_and(|(sequence, _)| sequence > context.confirmed_sequence)
            {
                if context.at_shop {
                    journal.map_values(|mut item| {
                        item.muted_until = tick;
                        item
                    });
                }
                continue;
            }
            *journal = TimerJournal::new();
            journal.push(0, context.slots[slot]);
        }
        self.changed();
    }

    /// Removes every wait the rejected order started.
    pub fn reject(&mut self, sequence: u32) -> bool {
        let mut removed = false;
        for journal in &mut self.journals {
            removed |= journal.reject(sequence);
        }
        for journal in &mut self.swaps {
            removed |= journal.reject(sequence);
        }
        if removed {
            self.changed();
        }
        removed
    }

    /// Ticks the hero's inventory `slot` stays muted by a swap, at tick `now`;
    /// absent for a courier and for slots outside the inventory.
    pub fn mute_left(&self, body: ControlledUnit, slot: ItemSlot, now: u32) -> Option<u32> {
        if body != ControlledUnit::Hero {
            return None;
        }
        let slot = usize::from(slot.0);
        if slot >= rules::INVENTORY_SLOTS {
            return None;
        }
        Some(
            self.swaps[slot]
                .current()
                .map_or(0, |p| p.muted_until.saturating_sub(now)),
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
        shared_left(self.journals[body.index()].current(), item, now, latency)
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
