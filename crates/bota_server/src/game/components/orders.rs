//! What an entity has been told to do, and what it has queued.

use std::collections::VecDeque;

use bota_core::rules::MAX_QUEUED_ORDERS;
use bota_proto::{EntityId, Order, Vec2};

use crate::game::{Entity, PendingCast};

/// The standing order an entity is following.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitOrder {
    /// Stand still. Takes on enemies that come near of its own accord.
    Idle,
    /// Stand still and take on nothing at all: what a stop order leaves
    /// behind.
    Stand,
    /// Stand still, attack what comes into range, never move.
    Hold,
    /// Walk to a position, ignoring enemies.
    Move {
        /// Destination.
        pos: Vec2,
    },
    /// Walk to a position, taking on enemies met on the way.
    AttackMove {
        /// Destination.
        pos: Vec2,
    },
    /// Attack one entity, following it while it stays visible.
    Attack {
        /// The target.
        target: Entity,
        /// Where the target was last seen by this entity's side.
        last_seen: Vec2,
    },
    /// Walk after one entity, taking on nobody.
    Follow {
        /// The entity being followed.
        target: Entity,
        /// Where it was last seen by this entity's side.
        last_seen: Vec2,
    },
}

/// The order in hand, a cast waiting on it, and how soon an enemy hero's
/// attack order may draw it again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orders {
    pub current: UnitOrder,
    /// For a hero: ticks before its attack order at an enemy hero may draw
    /// lane creeps again. For a tower: ticks before an enemy hero's attack
    /// order may draw it again. Zero when the next one does.
    pub cooldown: u32,
    /// For a tower: ticks before an attack order at an ally of the hero it
    /// shoots may take it off that hero again. Zero when the next one may.
    pub release_cooldown: u32,
    /// A cast ordered and not yet begun. Any order to the body takes it
    /// away.
    pub pending: Option<PendingCast>,
    /// An immediate cast ordered this tick, to go off beside whatever the
    /// body does. Nothing but its going off, or its caster being held or
    /// feared then, takes it away.
    pub immediate: Option<PendingCast>,
}

/// One order waiting its turn in a unit's queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Queued {
    /// The order, in the simulation's handles.
    pub order: Order,
    /// The number its seat gave it, to answer a refusal with when it comes
    /// up.
    pub seq: u32,
}

/// The orders a unit takes up one after another once it is done with the one
/// in hand: Dota's Shift-queue. Holds at most
/// [`MAX_QUEUED_ORDERS`](bota_core::rules::MAX_QUEUED_ORDERS).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderQueue {
    /// The next to be taken up first.
    waiting: VecDeque<Queued>,
}

impl OrderQueue {
    /// Puts an order last, or answers `false` when the queue is full.
    pub fn push(&mut self, queued: Queued) -> bool {
        if self.is_full() {
            return false;
        }
        self.waiting.push_back(queued);
        assert!(self.waiting.len() <= MAX_QUEUED_ORDERS);
        true
    }

    /// Takes the next order out.
    pub fn pop(&mut self) -> Option<Queued> {
        self.waiting.pop_front()
    }

    /// Whether another order would not fit.
    pub fn is_full(&self) -> bool {
        self.waiting.len() >= MAX_QUEUED_ORDERS
    }

    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    /// The orders waiting, the next first.
    pub fn iter(&self) -> impl Iterator<Item = &Queued> {
        self.waiting.iter()
    }

    /// Drops every order that names `target`.
    pub fn drop_aimed_at(&mut self, target: EntityId) {
        self.waiting.retain(|queued| {
            let mut named = false;
            let mut order = queued.order;
            order.visit_ids(&mut |id| named |= *id == target);
            !named
        });
    }
}
