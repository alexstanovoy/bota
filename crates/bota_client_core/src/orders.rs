//! The orders a client gives: what it asked for, what went out on the wire,
//! and what stands after the server's answers.

use bota_proto::{EntityId, ItemSlot, ItemView, Order, Target, UnitView};

use crate::session::{ClientSession, SessionError};

/// Which of a seat's two bodies an order drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControlledUnit {
    Hero,
    Courier,
}

impl ControlledUnit {
    /// The body an order's unit selector names.
    pub const fn of(unit: Option<EntityId>) -> Self {
        if unit.is_some() {
            Self::Courier
        } else {
            Self::Hero
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Hero => 0,
            Self::Courier => 1,
        }
    }
}

/// An order paired with the wire unit selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IssuedOrder {
    /// `None` selects the own hero; `Some` selects the current courier.
    pub unit: Option<EntityId>,
    pub order: Order,
    /// Whether it waits its turn in the unit's queue (Dota's Shift).
    pub queue: bool,
}

impl IssuedOrder {
    /// An order for the seat's own hero.
    pub const fn hero(order: Order) -> IssuedOrder {
        IssuedOrder {
            unit: None,
            order,
            queue: false,
        }
    }

    /// An order for the seat's courier, the unit `id`.
    pub const fn courier(id: EntityId, order: Order) -> IssuedOrder {
        IssuedOrder {
            unit: Some(id),
            order,
            queue: false,
        }
    }

    /// The same order, queued behind what the unit does and has queued.
    pub const fn queued(self) -> IssuedOrder {
        IssuedOrder {
            queue: true,
            ..self
        }
    }

    /// Whether it goes into its unit's queue: given queued, and of the
    /// orders that queue.
    pub const fn waits(&self) -> bool {
        self.queue && self.order.queues()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SentBodyOrder {
    sequence: u32,
    issued: IssuedOrder,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BodyRollback {
    sequence: u32,
    previous: Option<SentBodyOrder>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BodyOrders {
    current: Option<SentBodyOrder>,
    rollback: Option<BodyRollback>,
    /// Whether orders were queued behind `current` since it was given: the
    /// server's queue then holds more than the ledger knows, so the
    /// standing order is neither left unsent as a repeat nor given again.
    queued: bool,
}

/// Constant-size record of the standing order of each body, so a repeat can
/// be left unsent and a rejected order undone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderLedger {
    bodies: [BodyOrders; 2],
    last_seq: Option<u32>,
}

impl OrderLedger {
    /// Whether the order repeats the standing one of its body exactly. Only
    /// moves and attacks stand; anything else is a one-shot.
    pub fn repeats(&self, issued: IssuedOrder) -> bool {
        let body = self.body(issued.unit);
        is_standing(issued.order)
            && !issued.queue
            && !body.queued
            && body
                .current
                .is_some_and(|standing| standing.issued == issued)
    }

    /// Records an order after it was sent with a strictly increasing sequence.
    ///
    /// A one-shot that `ends_body` (see [`ends_body`]) clears the standing
    /// order without itself standing.
    pub fn record(
        &mut self,
        seq: u32,
        issued: IssuedOrder,
        ends_body: bool,
    ) -> Result<(), SessionError> {
        if let Some(previous) = self.last_seq
            && seq <= previous
        {
            return Err(SessionError::OrderSequence {
                incoming: seq,
                previous,
            });
        }
        self.last_seq = Some(seq);
        if issued.waits() {
            self.body_mut(issued.unit).queued = true;
        } else if is_standing(issued.order) {
            self.body_mut(issued.unit).transition(
                seq,
                issued.unit,
                Some(SentBodyOrder {
                    sequence: seq,
                    issued,
                }),
            );
        } else if ends_body {
            self.body_mut(issued.unit)
                .transition(seq, issued.unit, None);
        }
        Ok(())
    }

    /// Restores the preceding order when a body's newest order was rejected.
    pub fn reject(&mut self, seq: u32) -> bool {
        self.bodies.iter_mut().any(|body| body.reject(seq))
    }

    /// Clears one body's order after the body itself changed.
    pub fn clear(&mut self, unit: Option<EntityId>) {
        self.bodies[ControlledUnit::of(unit).index()] = BodyOrders::default();
    }

    /// The sequence and order standing for one body.
    pub fn standing(&self, unit: Option<EntityId>) -> Option<(u32, IssuedOrder)> {
        self.body(unit)
            .current
            .filter(|standing| standing.issued.unit == unit)
            .map(|standing| (standing.sequence, standing.issued))
    }

    /// The orders standing for the hero, then the courier, but for a body
    /// with orders queued behind its own: given again, it would empty the
    /// queue the server holds.
    pub fn standing_orders(&self) -> Vec<IssuedOrder> {
        self.bodies
            .iter()
            .filter(|body| !body.queued)
            .filter_map(|body| body.current.map(|standing| standing.issued))
            .collect()
    }

    /// The last sequence recorded, standing or not.
    pub const fn last_seq(&self) -> Option<u32> {
        self.last_seq
    }

    /// The ledger as the session's view now holds it: the orders of a dead
    /// or lost body dropped, those at a unit no longer in sight turned to
    /// the place it was last seen.
    pub fn reconciled(&self, session: &ClientSession) -> OrderLedger {
        let mut bodies = self.bodies;
        for body in &mut bodies {
            body.current = reconcile_standing(body.current, session);
            if let Some(rollback) = &mut body.rollback {
                rollback.previous = reconcile_standing(rollback.previous, session);
            }
        }
        OrderLedger {
            bodies,
            last_seq: self.last_seq,
        }
    }

    fn body(&self, unit: Option<EntityId>) -> &BodyOrders {
        &self.bodies[ControlledUnit::of(unit).index()]
    }

    fn body_mut(&mut self, unit: Option<EntityId>) -> &mut BodyOrders {
        &mut self.bodies[ControlledUnit::of(unit).index()]
    }
}

impl BodyOrders {
    fn transition(
        &mut self,
        sequence: u32,
        unit: Option<EntityId>,
        current: Option<SentBodyOrder>,
    ) {
        if self
            .current
            .is_some_and(|standing| standing.issued.unit != unit)
        {
            *self = Self::default();
        }
        self.rollback = Some(BodyRollback {
            sequence,
            previous: self.current,
        });
        self.current = current;
        self.queued = false;
    }

    fn reject(&mut self, sequence: u32) -> bool {
        let Some(rollback) = self
            .rollback
            .filter(|rollback| rollback.sequence == sequence)
        else {
            return false;
        };
        self.current = rollback.previous;
        self.rollback = None;
        true
    }
}

fn reconcile_standing(
    standing: Option<SentBodyOrder>,
    session: &ClientSession,
) -> Option<SentBodyOrder> {
    let mut standing = standing?;
    assert!(is_standing(standing.issued.order));
    standing.issued = reconcile_directive(session, standing.issued)?;
    assert!(is_standing(standing.issued.order));
    Some(standing)
}

/// A standing order as the session's view now holds it: dropped when its
/// body is dead, and when it follows or attacks a unit no longer in sight
/// turned to the place the unit was last seen there, or dropped if that is
/// not known to be its latest place.
pub fn reconcile_directive(
    session: &ClientSession,
    mut issued: IssuedOrder,
) -> Option<IssuedOrder> {
    if !body_alive(session, issued.unit) {
        return None;
    }
    let (Order::Attack { target: aim } | Order::Move { target: aim }) = &mut issued.order else {
        return Some(issued);
    };
    let Target::Unit(target) = *aim else {
        return Some(issued);
    };
    // A truncated view cannot prove a visibility loss: the session owns the
    // complete validated snapshot, sorted by full-generation handle.
    let current = session.current().expect("a snapshot");
    if current.unit(target).is_some_and(unit_alive) {
        return Some(issued);
    }
    let observed = session.entity(target)?;
    // Gaps and same-tick deaths can hide a newer server last-seen position.
    if observed.last_seen_tick.checked_add(1) != Some(current.tick)
        || observed
            .last_death
            .is_some_and(|death| death.tick >= observed.last_seen_tick)
    {
        return None;
    }
    *aim = Target::Pos(observed.unit.pos);
    Some(issued)
}

/// Whether the own body an order's selector names stands.
pub fn body_alive(session: &ClientSession, unit: Option<EntityId>) -> bool {
    match unit {
        None => session.own_hero().is_some_and(unit_alive),
        Some(id) => session
            .own_courier()
            .is_some_and(|courier| courier.id == id && unit_alive(courier)),
    }
}

fn unit_alive(unit: &UnitView) -> bool {
    unit.hp > 0
}

const fn is_standing(order: Order) -> bool {
    matches!(order, Order::Move { .. } | Order::Attack { .. })
}

/// The item a unit holds in a slot, the scroll slot included.
pub(crate) fn item_in(unit: &UnitView, slot: ItemSlot) -> Option<&ItemView> {
    if slot == ItemSlot::TOWN_PORTAL {
        unit.town_portal.as_ref()
    } else {
        unit.items.get(usize::from(slot.0))?.as_ref()
    }
}

/// Whether an order ends its body's standing move or attack by
/// [`bota_core::ends_standing_order`], reading the ability or item in the slot
/// it names off the body as the session last saw it.
pub fn ends_body(session: &ClientSession, issued: IssuedOrder) -> bool {
    let body = match issued.unit {
        None => session.own_hero(),
        Some(id) => session.own_courier().filter(|courier| courier.id == id),
    };
    let (ability, item) = match (issued.order, body) {
        (Order::Cast { slot, .. }, Some(unit)) => {
            (unit.abilities.get(usize::from(slot.0)).map(|a| a.id), None)
        }
        (Order::Use { slot, .. }, Some(unit)) => (None, item_in(unit, slot).map(|i| i.id)),
        _ => (None, None),
    };
    bota_core::ends_standing_order(issued.order, ability, item)
}
