//! Orders queued behind the one in hand, taken up one after another as
//! Dota's Shift-queue takes them.

use bota_core::rules::MAX_QUEUED_ORDERS;

use crate::game::{
    ActionState, Command, Entity, Errand, Event, Queued, RefusedOrder, UnitOrder, World, wire_id,
};

impl World {
    /// Whether a unit would take a queued order up at once: nothing queued,
    /// and done with the order in hand.
    pub fn free_to_take_up(&self, unit: Entity) -> bool {
        self.queue.get(unit).is_none_or(|queue| queue.is_empty()) && self.order_done(unit)
    }

    /// Whether a unit is done with the order in hand: no cast waiting, going
    /// off, channelled or in its backswing, no item or courier errand under
    /// way, and an order that has nothing left to do. A hold, a walk in a direction, and an
    /// attack or a follow while the target stands, are never done.
    pub fn order_done(&self, unit: Entity) -> bool {
        let Some(orders) = self.orders.get(unit) else {
            return true;
        };
        if orders.pending.is_some() || orders.immediate.is_some() {
            return false;
        }
        let casting = matches!(
            self.action.get(unit).map(|action| action.state),
            Some(ActionState::CastAbility { .. } | ActionState::UseItem { .. })
        );
        if casting
            || self.handling.contains(unit)
            || self
                .errand
                .get(unit)
                .is_some_and(|errand| *errand != Errand::None)
        {
            return false;
        }
        match orders.current {
            UnitOrder::Idle | UnitOrder::Stand => true,
            UnitOrder::Move { pos } => self.walked_to(unit, pos),
            UnitOrder::AttackMove { pos } => {
                self.walked_to(unit, pos) && self.target_of(unit).is_none()
            }
            UnitOrder::Hold
            | UnitOrder::Direction { .. }
            | UnitOrder::Attack { .. }
            | UnitOrder::Follow { .. } => false,
        }
    }

    /// Whether a walker stands at the end of its walk to `pos`: on it, or as
    /// near as the ground lets it come.
    fn walked_to(&self, unit: Entity, pos: bota_proto::Vec2) -> bool {
        self.pos(unit) == Some(pos)
            || self
                .route
                .get(unit)
                .is_some_and(|route| route.done && route.goal == Some(pos))
    }

    /// Puts a queued order last in its unit's queue, or refuses it when the
    /// queue is full.
    pub(crate) fn enqueue(&mut self, unit: Entity, cmd: &Command) {
        debug_assert!(cmd.queued());
        let queued = Queued {
            order: cmd.order,
            seq: cmd.seq,
        };
        let room = match self.queue.get_mut(unit) {
            Some(queue) => queue.push(queued),
            None => {
                let mut queue = crate::game::OrderQueue::default();
                let room = queue.push(queued);
                self.queue.insert(unit, queue);
                room
            }
        };
        if !room {
            self.refused.push(RefusedOrder {
                slot: cmd.slot,
                seq: cmd.seq,
                reason: bota_proto::RejectReason::QueueFull,
            });
        }
    }

    /// Every unit done with the order in hand takes up the next one it has
    /// queued, checked again first: one that no longer holds is refused and
    /// the one after it taken up, and one done at once lets the next follow
    /// on the same tick.
    pub(crate) fn take_up_queues(&mut self, events: &mut Vec<Event>) {
        let entities = self.take_entity_snapshot();
        for unit in entities.iter().copied() {
            for _ in 0..=MAX_QUEUED_ORDERS {
                if self.queue.get(unit).is_none_or(|queue| queue.is_empty())
                    || !self.order_done(unit)
                {
                    break;
                }
                let Some(&slot) = self.owner.get(unit) else {
                    self.queue.remove(unit);
                    break;
                };
                let next = self
                    .queue
                    .get_mut(unit)
                    .and_then(|queue| queue.pop())
                    .expect("the queue was just seen to hold an order");
                match self.validate_order(slot, Some(wire_id(unit)), &next.order) {
                    Ok(()) => self.carry_out(slot, unit, next.order, events),
                    Err(reason) => self.refused.push(RefusedOrder {
                        slot,
                        seq: next.seq,
                        reason,
                    }),
                }
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Whether a unit has orders queued.
    pub fn has_queued(&self, unit: Entity) -> bool {
        self.queue.get(unit).is_some_and(|queue| !queue.is_empty())
    }
}
