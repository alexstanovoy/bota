//! What making an illusion of a hero does to the other side's aims.

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
        let named = wire_id(hero);
        let entities = self.take_entity_snapshot();
        for unit in entities.iter().copied() {
            if self.team.get(unit).is_none_or(|team| *team == side) {
                continue;
            }
            if let Some(orders) = self.orders.get_mut(unit) {
                if matches!(
                    orders.current,
                    UnitOrder::Attack { target, .. } | UnitOrder::Follow { target, .. }
                        if target == hero
                ) {
                    orders.current = UnitOrder::Idle;
                }
                let aimed = |cast: &PendingCast| cast.target() == bota_proto::Target::Unit(named);
                if orders.pending.as_ref().is_some_and(aimed) {
                    orders.pending = None;
                }
            }
            if self.target_of(unit) == Some(hero) {
                self.target.remove(unit);
            }
            if let Some(queue) = self.queue.get_mut(unit) {
                queue.drop_aimed_at(named);
            }
        }
        self.recycle_entity_snapshot(entities);
    }
}
