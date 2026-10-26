//! Wards: standing them, and taking them away when their time is up.

use bota_core::{UnitDef, UseSpec, item_def, rules};
use bota_proto::{Inflictor, ItemId, Target};

use crate::game::{Entity, World};

impl World {
    /// Runs down what stands for a time, and takes away whatever has run out.
    pub fn tick_expiries(&mut self) {
        let lapsed = std::mem::take(&mut self.lapsed_lives);
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            if lapsed.contains(&entity) {
                self.despawn(entity);
                continue;
            }
            let Some(left) = self.expiry.get_mut(entity) else {
                continue;
            };
            left.ticks_left = left.ticks_left.saturating_sub(1);
            if left.ticks_left > 0 {
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
        (item, def): (ItemId, &'static UnitDef),
        ticks: u32,
        range: i32,
    ) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let (Some(side), Some(from)) = (self.team.get(user).copied(), self.pos(user)) else {
            return false;
        };
        if !from.within(pos, rules::units(range)) || !self.clearance.stands_clear(pos) {
            return false;
        }
        let by = (Some(user), Some(Inflictor::Item(item)));
        self.spawn_ward(def, (side, pos), ticks, by);
        self.settle();
        true
    }

    /// Plants the kind of ward a Ward Dispenser in a slot is switched to, by
    /// the entry of that ward, and takes one out of the dispenser.
    pub fn dispense_ward(&mut self, user: Entity, slot: usize, target: Target) -> bool {
        let Some(stack) = self.held_in_bag(user, slot) else {
            return false;
        };
        let ward = stack.active_ward();
        let Some(UseSpec::Ward { unit, ticks, range }) = item_def(ward).map(|def| def.effect)
        else {
            return false;
        };
        if !self.stand_ward(user, target, (ward, unit), ticks, range) {
            return false;
        }
        if let Some(held) = self
            .inventory
            .get_mut(user)
            .and_then(|bag| bag.slot_mut(slot))
            .and_then(Option::as_mut)
        {
            held.spend_ward();
        }
        true
    }

    /// Switches the kind of ward the Ward Dispenser in a slot plants next.
    pub fn switch_ward(&mut self, user: Entity, slot: usize) {
        if let Some(held) = self
            .inventory
            .get_mut(user)
            .and_then(|bag| bag.slot_mut(slot))
            .and_then(Option::as_mut)
            .filter(|held| held.id == ItemId::WARD_DISPENSER)
        {
            held.switch_ward();
        }
    }
}
