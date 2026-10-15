//! The orders a client gives: what it asked for, what went out on the wire,
//! and what stands after the server's answers.

use bota_proto::{ClientMsg, EntityId, Order, RejectReason, Target, UnitView};

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
}

/// An order as it goes to the server: the caller transmits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutgoingOrder {
    /// Sequence number, above every one before it; a rejection carries it back.
    pub seq: u32,
    pub unit: Option<EntityId>,
    pub order: Order,
    /// The tick of the snapshot it was decided on, from which the server
    /// measures the order delay.
    pub seen: u32,
}

impl OutgoingOrder {
    /// The message that carries it.
    pub const fn message(self) -> ClientMsg {
        ClientMsg::Order {
            seq: self.seq,
            unit: self.unit,
            order: self.order,
            seen: Some(self.seen),
        }
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
        is_standing(issued.order)
            && self
                .body(issued.unit)
                .current
                .is_some_and(|standing| standing.issued == issued)
    }

    /// Records an order after it was sent with a strictly increasing sequence.
    ///
    /// A one-shot that works the body (a cast, a use, a put or a take)
    /// clears the standing order without itself standing, unless `keeps_body`
    /// says it leaves the body alone.
    pub fn record(
        &mut self,
        seq: u32,
        issued: IssuedOrder,
        keeps_body: bool,
    ) -> Result<(), SessionError> {
        let interrupts = works_body(issued.order) && !keeps_body;
        if let Some(previous) = self.last_seq
            && seq <= previous
        {
            return Err(SessionError::OrderSequence {
                incoming: seq,
                previous,
            });
        }
        self.last_seq = Some(seq);
        if is_standing(issued.order) {
            self.body_mut(issued.unit).transition(
                seq,
                issued.unit,
                Some(SentBodyOrder {
                    sequence: seq,
                    issued,
                }),
            );
        } else if interrupts {
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

    /// The greatest sequence among the standing orders.
    pub fn newest_standing(&self) -> Option<u32> {
        self.bodies
            .iter()
            .filter_map(|body| body.current.map(|standing| standing.sequence))
            .max()
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
    let target = match issued.order {
        Order::Attack {
            target: Target::Unit(target),
        }
        | Order::Move {
            target: Target::Unit(target),
        } => target,
        _ => return Some(issued),
    };
    // A truncated view cannot prove a visibility loss: the session owns the
    // complete validated snapshot, sorted by full-generation handle.
    let current = session.current().expect("a snapshot");
    if current
        .units
        .binary_search_by_key(&target, |unit| unit.id)
        .ok()
        .is_some_and(|index| unit_alive(&current.units[index]))
    {
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
    let position = observed.unit.pos;
    issued.order = match issued.order {
        Order::Attack { .. } => Order::Attack {
            target: Target::Pos(position),
        },
        Order::Move { .. } => Order::Move {
            target: Target::Pos(position),
        },
        _ => unreachable!("checked unit-target directive"),
    };
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

const fn works_body(order: Order) -> bool {
    matches!(
        order,
        Order::Cast { .. } | Order::Use { .. } | Order::Put { .. } | Order::Take { .. }
    )
}

/// What a refusal is called in plain words.
pub fn refusal(reason: RejectReason) -> &'static str {
    use RejectReason as Why;
    match reason {
        Why::HeroDead => "your hero is not standing",
        Why::UnknownTarget => "no such target",
        Why::WrongTargetKind => "not aimed at what it takes",
        Why::OnCooldown => "still on cooldown",
        Why::NotEnoughMana => "not enough mana",
        Why::NotEnoughGold => "not enough gold",
        Why::EmptySlot => "that slot is empty",
        Why::NotCastable => "it works on its own",
        Why::NotLearned => "no points in it yet",
        Why::NoCharges => "no charges left",
        Why::NotReady => "it is not working yet",
        Why::NotYourUnit => "you do not drive that unit",
        Why::UnknownItem => "the shop does not sell that",
        Why::NotAtShop => "only at the shop",
        Why::InventoryFull => "no room for it",
        Why::Disabled => "you cannot act right now",
        Why::NotYourItem => "not yours to sell",
        Why::ClosedGround => "nothing can lie there",
        Why::NotInBag => "not carried in the bag",
        Why::NoCheats => "cheats are off in this match",
        Why::BadCheat => "the cheat is out of bounds",
        Why::OutOfStock => "the shop has none left",
        Why::CannotLearn => "that talent cannot be learned now",
        Why::Unsellable => "a free scroll sells for nothing",
    }
}
