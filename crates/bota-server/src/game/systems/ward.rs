//! Wards: standing them, and taking them away when their time is up.

use bota_proto::Target;

use crate::game::rules;
use crate::game::{Entity, Expiry, UnitDef, World};

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
    /// The spot has to be ground its user could walk on, within reach of where
    /// that user stands. What stands there afterwards takes no room: it is
    /// walked through rather than round.
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
        if !from.within(pos, rules::units(range)) || !self.grid.walkable(pos) {
            return false;
        }
        let ward = self.spawn_unit(def, side, pos);
        self.expiry.insert(ward, Expiry { ticks_left: ticks });
        self.settle();
        true
    }
}
