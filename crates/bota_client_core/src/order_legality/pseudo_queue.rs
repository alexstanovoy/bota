//! Tracks orders held behind a cast point, from a seat's own issued orders and snapshots.
//!
//! Exact for next-tick orders. Delayed input may meet a different cast; rejections and
//! subsequent observations reconcile the record. Shift queues are told by UnitView instead.

use bota_core::{order_halts as is_halt, order_is_cast as is_cast, rules::MAX_QUEUED_ORDERS};
use bota_proto::{AbilityId, CastView, EntityId, ItemSlot, Order, StatusFlags, UnitView};

use super::reservations::order_mana_is_affordable;
use crate::{ControlledUnit, IssuedOrder};

/// One order a body owes: its sequence, the order, and the cast point it shows once it is
/// taken up, if it shows one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owed {
    pub seq: u32,
    pub order: Order,
    /// Public mana, including orders ahead, covered its cost when it was issued.
    mana_affordable_at_issue: bool,
    /// The cast point taking it up shows: [`CastView::ability`], absent for an item's use.
    /// Absent for an order that shows none (a use or cast with no cast point, or no cast).
    shows: Option<Option<AbilityId>>,
}

impl Owed {
    /// Whether it is a cast or a use, which waits in turn behind other casts.
    pub const fn casts(&self) -> bool {
        is_cast(self.order)
    }
}

/// What a cast point owes one body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwedOrders {
    /// The cast whose cast point the body stands in, when it is known from the seat's orders.
    pub in_hand: Option<Owed>,
    /// The orders waiting for the cast, the next first: casts only, or one other order.
    waiting: [Option<Owed>; MAX_QUEUED_ORDERS],
    length: usize,
}

impl OwedOrders {
    /// Orders waiting behind the current cast, in execution order.
    pub fn waiting(&self) -> impl Iterator<Item = &Owed> {
        self.waiting[..self.length].iter().flatten()
    }

    pub const fn len(&self) -> usize {
        self.length
    }
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    fn clear(&mut self) {
        self.waiting.fill(None);
        self.length = 0;
    }

    fn push(&mut self, owed: Owed) {
        if let Some(place) = self.waiting.get_mut(self.length) {
            *place = Some(owed);
            self.length += 1;
        }
    }

    fn retain(&mut self, keep: impl Fn(&Owed) -> bool) {
        let mut length = 0;
        for index in 0..self.length {
            if let Some(owed) = self.waiting[index].take().filter(&keep) {
                self.waiting[length] = Some(owed);
                length += 1;
            }
        }
        self.length = length;
    }

    /// The item slot the use in hand casts.
    pub fn item_in_hand(&self) -> Option<ItemSlot> {
        match self.in_hand?.order {
            Order::Use { slot, .. } => Some(slot),
            _ => None,
        }
    }

    /// Whether the orders waiting are casts only: a cast given now waits behind them, where it
    /// would replace any other order.
    pub fn casts_waiting(&self) -> bool {
        self.waiting().all(Owed::casts)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BodyRecord {
    /// The body the record is of; another body owes nothing of it.
    body: Option<EntityId>,
    /// The cast point the last snapshot showed.
    cast: Option<CastView>,
    /// A cast or use carried out at once whose cast point has not been seen to begin.
    taken: Option<Owed>,
    owed: OwedOrders,
    /// The snapshot tick the body's last order to its body was given on, and the record as it
    /// stood before: the last order to a body on a tick replaces the earlier ones, unanswered.
    before: Option<(u32, Option<Owed>, OwedOrders)>,
}

/// The pseudo-queues of a seat's hero and courier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PseudoQueue {
    bodies: [BodyRecord; 2],
}

impl PseudoQueue {
    /// What a cast point owes `unit`.
    pub fn owed(&self, unit: ControlledUnit) -> &OwedOrders {
        &self.bodies[unit.index()].owed
    }

    /// Whether the order `seq` waits for a cast point.
    pub fn waits(&self, seq: u32) -> bool {
        self.bodies
            .iter()
            .any(|body| body.owed.waiting().any(|owed| owed.seq == seq))
    }

    pub fn waiting_mana_was_affordable(&self, seq: u32) -> bool {
        self.bodies.iter().any(|body| {
            body.owed
                .waiting()
                .any(|owed| owed.seq == seq && owed.mana_affordable_at_issue)
        })
    }

    /// Records the order `seq` given to `body` on the snapshot of `tick`, as the body stands in
    /// it. Orders to the unit's Shift queue are read from its view, not recorded here.
    pub fn give(&mut self, seq: u32, issued: IssuedOrder, tick: u32, body: &UnitView) {
        if issued.queue || !bota_core::order_queues(issued.order) {
            return;
        }
        let record = &mut self.bodies[ControlledUnit::of(issued.unit).index()];
        record.follow(body.id);
        match &record.before {
            Some((given, taken, owed)) if *given == tick => {
                record.taken = *taken;
                record.owed = owed.clone();
            }
            _ => record.before = Some((tick, record.taken, record.owed.clone())),
        }
        if immediate(body, issued.order) {
            return;
        }
        let owed = Owed {
            seq,
            order: issued.order,
            mana_affordable_at_issue: order_mana_is_affordable(body, &record.owed, issued.order),
            shows: shows(body, issued.order),
        };
        if is_halt(issued.order) {
            record.taken = None;
            record.owed = OwedOrders::default();
        } else if body.cast.is_some() {
            record.owed.wait(owed);
        } else if has_status(body, StatusFlags::CHANNELLING) && ignores_channel(body, owed.order) {
            record.taken = owed.casts().then_some(owed);
        } else {
            record.owed.clear();
            record.taken = owed.casts().then_some(owed);
        }
    }

    /// Forgets the refused order `seq`.
    pub fn reject(&mut self, seq: u32) {
        for record in &mut self.bodies {
            record.owed.retain(|owed| owed.seq != seq);
            if record.taken.is_some_and(|taken| taken.seq == seq) {
                record.taken = None;
            }
        }
    }

    /// Follows each body into the snapshot it now stands in: a cast point that begins takes up
    /// the cast given at once, or else the first waiting one it can be (those before it went
    /// off without a cast point of their own); one that is over leaves nothing in hand, and a
    /// move, an attack or the like waiting for it runs.
    pub fn observe(&mut self, hero: Option<&UnitView>, courier: Option<&UnitView>) {
        for (record, body) in self.bodies.iter_mut().zip([hero, courier]) {
            let Some(body) = body.filter(|body| body.hp > 0) else {
                *record = BodyRecord::default();
                continue;
            };
            record.follow(body.id);
            let began = match (record.cast, body.cast) {
                (was, None) => {
                    record.owed.in_hand = None;
                    // A cast that went off, not one a stun broke off, has the body take up
                    // what waits for it: another order runs at once.
                    let taken_up = was.is_some()
                        && !has_status(body, StatusFlags::STUNNED | StatusFlags::FEARED);
                    if taken_up && !record.owed.casts_waiting() {
                        record.owed.clear();
                    }
                    None
                }
                (None, Some(now)) => Some(now),
                (Some(was), Some(now)) => (now.goes_off_in > was.goes_off_in).then_some(now),
            };
            if let Some(cast) = began {
                record.begin(cast);
            }
            record.cast = body.cast;
        }
    }
}

impl BodyRecord {
    /// Forgets what another body owed.
    fn follow(&mut self, body: EntityId) {
        if self.body != Some(body) {
            *self = Self {
                body: Some(body),
                ..Self::default()
            };
        }
    }

    fn begin(&mut self, cast: CastView) {
        let shown = Some(cast.ability);
        let mut rest = OwedOrders::default();
        let mut in_hand = None;
        for owed in self
            .taken
            .take()
            .into_iter()
            .chain(self.owed.waiting().copied())
        {
            if in_hand.is_some() {
                rest.push(owed);
            } else if owed.shows == shown {
                in_hand = Some(owed);
            }
        }
        rest.in_hand = in_hand;
        self.owed = rest;
    }
}

impl OwedOrders {
    /// An order given in a cast point: a cast waits behind the casts waiting, anything else
    /// replaces what waits; a cast that finds the queue full is refused.
    fn wait(&mut self, owed: Owed) {
        if !(owed.casts() && self.casts_waiting()) {
            self.clear();
        }
        if self.length < MAX_QUEUED_ORDERS {
            self.push(owed);
        }
    }
}

const fn has_status(unit: &UnitView, flag: u16) -> bool {
    unit.statuses.bits & flag != 0
}

/// The ability or item definition a cast or use names on `body`.
fn cast_def(body: &UnitView, order: Order) -> Option<CastDef> {
    match order {
        Order::Cast { slot, .. } => {
            let id = body.abilities.get(usize::from(slot.0))?.id;
            bota_core::ability_def(id).map(|def| CastDef {
                ability: Some(id),
                immediate: def.immediate,
                ignores_channel: def.ignores_channel,
                point: def.point,
            })
        }
        Order::Use { slot, .. } => {
            let held = if slot == ItemSlot::TOWN_PORTAL {
                body.town_portal.as_ref()
            } else {
                body.items.get(usize::from(slot.0))?.as_ref()
            }?;
            bota_core::item_def(held.id).map(|def| CastDef {
                ability: None,
                immediate: def.immediate,
                ignores_channel: def.ignores_channel,
                point: def.point,
            })
        }
        _ => None,
    }
}

struct CastDef {
    ability: Option<AbilityId>,
    immediate: bool,
    ignores_channel: bool,
    point: u32,
}

/// Whether what an order casts goes off beside whatever the body does: Dota's
/// `DOTA_ABILITY_BEHAVIOR_IMMEDIATE`, which neither waits nor breaks anything off.
fn immediate(body: &UnitView, order: Order) -> bool {
    cast_def(body, order).is_some_and(|def| def.immediate)
}

fn ignores_channel(body: &UnitView, order: Order) -> bool {
    cast_def(body, order).is_some_and(|def| def.ignores_channel)
}

fn shows(body: &UnitView, order: Order) -> Option<Option<AbilityId>> {
    cast_def(body, order)
        .filter(|def| def.point > 0)
        .map(|def| def.ability)
}
