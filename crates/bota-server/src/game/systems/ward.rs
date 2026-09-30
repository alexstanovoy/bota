//! Wards: standing them, and taking them away when their time is up.

use bota_proto::Target;

use crate::game::rules;
use crate::game::{Entity, UnitDef, World};

impl World {
    /// Runs down what stands for a time, and takes away whatever has run out.
    pub fn tick_expiries(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(mut left) = self.expiry.get(entity).copied() else {
                continue;
            };
            left.ticks_left = left.ticks_left.saturating_sub(1);
            if left.ticks_left > 0 {
                self.expiry.insert(entity, left);
                continue;
            }
            self.expiry.remove(entity);
            self.despawn(entity);
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Stands a ward at the spot an item was aimed at.
    ///
    /// The spot has to be clear walkable ground within `range` of the user.
    /// The ward has no hull: it takes no room.
    pub fn stand_ward(
        &mut self,
        user: Entity,
        target: Target,
        def: &'static UnitDef,
        ticks: u32,
        range: i32,
    ) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let (Some(side), Some(from)) = (
            self.team.get(user).copied(),
            self.transform.get(user).map(|t| t.pos),
        ) else {
            return false;
        };
        if !from.within(pos, rules::units(range)) || !self.clearance.stands_clear(pos) {
            return false;
        }
        self.spawn_ward(def, side, pos, ticks);
        self.settle();
        true
    }
}
