//! Item errands: walking an item out of a bag, into another, or off the
//! ground.

use bota_core::rules;
use bota_proto::{Angle, EntityId, Target, Vec2};

use crate::game::{
    BAG_SLOTS, Entity, Handling, ItemStack, Loot, Transform, UnitOrder, Visibility, World,
};

impl World {
    /// Starts walking an item out of one of a unit's bag slots.
    ///
    /// Aimed at a point the item will be laid there; aimed at nothing, at the
    /// unit's own feet; aimed at an allied unit with a bag, handed into it.
    /// The walking and the doing live with [`World::tick_handling`].
    pub fn put_item(&mut self, unit: Entity, slot: usize, target: Target) -> bool {
        if slot >= BAG_SLOTS || self.held_in_bag(unit, slot).is_none() {
            return false;
        }
        let errand = match target {
            Target::None => {
                let Some(pos) = self.pos(unit) else {
                    return false;
                };
                Handling::PutAt { slot, pos }
            }
            Target::Pos(pos) => Handling::PutAt { slot, pos },
            Target::Unit(target) => {
                let Some(to) = self.of_wire(target) else {
                    return false;
                };
                if to == unit
                    || self.team.get(to) != self.team.get(unit)
                    || self.inventory.get(to).is_none()
                {
                    return false;
                }
                Handling::PutTo { slot, to }
            }
        };
        self.handling.insert(unit, errand);
        true
    }

    /// Starts walking a unit over to a ground item to take it.
    pub fn take_item(&mut self, unit: Entity, target: EntityId) -> bool {
        let Some(mark) = self.of_wire(target) else {
            return false;
        };
        if self.loot.get(mark).is_none() || self.inventory.get(unit).is_none() {
            return false;
        }
        self.handling.insert(unit, Handling::Take { loot: mark });
        true
    }

    /// Carries every item errand one tick on.
    ///
    /// A unit not yet in reach walks; one in reach does the thing and is left
    /// standing. An errand that comes to nothing (the slot emptied, the target
    /// fallen or full, the item taken by somebody quicker) ends where it is
    /// found, the same way.
    pub fn tick_handling(&mut self) {
        let entities = self.take_entity_snapshot();
        for unit in entities.iter().copied() {
            let Some(errand) = self.handling.get(unit).copied() else {
                continue;
            };
            if !self.alive(unit) {
                self.handling.remove(unit);
                continue;
            }
            let done = match errand {
                Handling::PutAt { slot, pos } => self.lay_at(unit, slot, pos),
                Handling::PutTo { slot, to } => self.hand_to(unit, slot, to),
                Handling::Take { loot } => self.take_from_ground(unit, loot),
            };
            if done {
                self.handling.remove(unit);
                self.set_order(unit, UnitOrder::Stand);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Walks a unit within reach of a spot and lays the stack there.
    fn lay_at(&mut self, unit: Entity, slot: usize, pos: Vec2) -> bool {
        if self.held_in_bag(unit, slot).is_none() {
            return true;
        }
        let Some(from) = self.pos(unit) else {
            return true;
        };
        if !from.within(pos, rules::units(rules::PUT_ITEM_RANGE)) {
            self.set_order(unit, UnitOrder::Move { pos });
            return false;
        }
        if !self.clearance.stands_clear(pos) {
            return true;
        }
        let Some(stack) = self.take_from_bag(unit, slot) else {
            return true;
        };
        self.lay_loot(stack, pos);
        true
    }

    /// Walks a unit within reach of an ally and hands the stack into its bag.
    fn hand_to(&mut self, unit: Entity, slot: usize, to: Entity) -> bool {
        if self.held_in_bag(unit, slot).is_none() {
            return true;
        }
        if !self.alive(to) || self.team.get(to) != self.team.get(unit) {
            return true;
        }
        let (Some(from), Some(at)) = (self.pos(unit), self.pos(to)) else {
            return true;
        };
        if !from.within(at, rules::units(rules::PUT_ITEM_RANGE)) {
            self.set_order(unit, UnitOrder::Move { pos: at });
            return false;
        }
        let Some(stack) = self.take_from_bag(unit, slot) else {
            return true;
        };
        let handed = ItemStack {
            touched: true,
            ..stack
        };
        let taken = self
            .inventory
            .get_mut(to)
            .is_some_and(|bag| bag.stow(handed));
        if !taken
            && let Some(held) = self
                .inventory
                .get_mut(unit)
                .and_then(|bag| bag.slot_mut(slot))
        {
            *held = Some(stack);
        }
        true
    }

    /// Walks a unit over to a ground item and takes it into the first free
    /// bag slot, a scroll into a hero's town portal slot.
    fn take_from_ground(&mut self, unit: Entity, loot: Entity) -> bool {
        let Some(Loot(stack)) = self.loot.get(loot).copied() else {
            return true;
        };
        let Some(spot) = self.pos(loot) else {
            return true;
        };
        let Some(from) = self.pos(unit) else {
            return true;
        };
        if !from.within(spot, rules::units(rules::TAKE_ITEM_RANGE)) {
            self.set_order(unit, UnitOrder::Move { pos: spot });
            return false;
        }
        let taken = self.inventory.get_mut(unit).is_some_and(|bag| {
            bag.stow(ItemStack {
                touched: true,
                ..stack
            })
        });
        if !taken {
            return true;
        }
        self.loot.remove(loot);
        self.despawn(loot);
        true
    }

    /// Stands a ground item up holding a stack.
    pub fn lay_loot(&mut self, stack: ItemStack, pos: Vec2) -> Entity {
        let entity = self.spawn();
        self.transform.insert(
            entity,
            Transform {
                pos,
                facing: Angle::default(),
            },
        );
        self.loot.insert(
            entity,
            Loot(ItemStack {
                touched: true,
                ..stack
            }),
        );
        // A row in the sight table is what makes it something sides can see.
        self.visibility.insert(entity, Visibility::NONE);
        entity
    }

    /// The stack in one of a unit's own bag slots, read in place.
    pub(crate) fn held_in_bag(&self, unit: Entity, slot: usize) -> Option<ItemStack> {
        self.inventory
            .get(unit)
            .and_then(|bag| bag.slot(slot).copied().flatten())
    }

    /// Takes the stack out of one of a unit's own bag slots.
    pub(crate) fn take_from_bag(&mut self, unit: Entity, slot: usize) -> Option<ItemStack> {
        self.inventory.get_mut(unit)?.slot_mut(slot)?.take()
    }
}
