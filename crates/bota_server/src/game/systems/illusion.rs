//! What making an illusion of a hero, or handing a unit to another side,
//! does to the aims at it.

use bota_proto::Team;

use crate::game::{Entity, PendingCast, UnitOrder, World, wire_id};

impl World {
    /// Calls off what the other side aims at a hero an illusion of it was
    /// just made of, as Dota's illusion makers do: attack and follow orders
    /// and casts waiting on it, whom its units are set on, and the queued
    /// orders that name it. None of them follows the hero under the name
    /// the other side is told for it afresh.
    pub fn call_off_aims_at(&mut self, hero: Entity) {
        let Some(&side) = self.team.get(hero) else {
            return;
        };
        self.call_off_aims(hero, |team| team != side);
    }

    /// Calls off every aim at a unit of the units whose side `of` takes: the
    /// orders and casts waiting on it, whom they are set on, and the queued
    /// orders that name it.
    pub(crate) fn call_off_aims(&mut self, mark: Entity, of: impl Fn(Team) -> bool) {
        let named = wire_id(mark);
        let entities = self.take_entity_snapshot();
        for unit in entities.iter().copied() {
            if self.team.get(unit).is_none_or(|team| !of(*team)) {
                continue;
            }
            if let Some(orders) = self.orders.get_mut(unit) {
                if matches!(
                    orders.current,
                    UnitOrder::Attack { target, .. } | UnitOrder::Follow { target, .. }
                        if target == mark
                ) {
                    orders.current = UnitOrder::Idle;
                }
                let aimed = |cast: &PendingCast| cast.target() == bota_proto::Target::Unit(named);
                if orders.pending.as_ref().is_some_and(aimed) {
                    orders.pending = None;
                }
            }
            if self.target_of(unit) == Some(mark) {
                self.target.remove(unit);
            }
            for queue in [self.queue.get_mut(unit), self.waiting.get_mut(unit)]
                .into_iter()
                .flatten()
            {
                queue.drop_aimed_at(named);
            }
        }
        self.recycle_entity_snapshot(entities);
    }
}
