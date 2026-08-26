//! Item errands: walking an item out of a bag, into another, or off the
//! ground.

use bota_proto::{Target, Vec2};

use crate::game::{
    BAG_SLOTS, Entity, Handling, ItemStack, Loot, Transform, UnitOrder, Visibility, World, rules,
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
                let Some(pos) = self.transform.get(unit).map(|t| t.pos) else {
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
    pub fn take_item(&mut self, unit: Entity, target: bota_proto::EntityId) -> bool {
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
    /// A unit not yet in reach walks; one in reach does the thing. An errand
    /// that comes to nothing — the slot emptied, the target fallen or full,
    /// the item taken by somebody quicker — is put down where it is found.
    pub fn tick_handling(&mut self) {
        for unit in self.entities.iter().collect::<Vec<_>>() {
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
    }

    /// Walks a unit within reach of a spot and lays the stack there.
    fn lay_at(&mut self, unit: Entity, slot: usize, pos: Vec2) -> bool {
        if self.held_in_bag(unit, slot).is_none() {
            return true;
        }
        let Some(from) = self.transform.get(unit).map(|t| t.pos) else {
            return true;
        };
        if !from.within(pos, rules::units(rules::PUT_ITEM_RANGE)) {
            self.set_order(unit, UnitOrder::Move { pos });
            return false;
        }
        if !self.grid.walkable(pos) {
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
        let (Some(from), Some(at)) = (
            self.transform.get(unit).map(|t| t.pos),
            self.transform.get(to).map(|t| t.pos),
        ) else {
            return true;
        };
        if !from.within(at, rules::units(rules::PUT_ITEM_RANGE)) {
            self.set_order(unit, UnitOrder::Move { pos: at });
            return false;
        }
        let has_room = self
            .inventory
            .get(to)
            .is_some_and(|bag| bag.slots.iter().any(|held| held.is_none()));
        if !has_room {
            return true;
        }
        let Some(stack) = self.take_from_bag(unit, slot) else {
            return true;
        };
        if let Some(bag) = self.inventory.get_mut(to)
            && let Some(free) = bag.slots.iter_mut().find(|held| held.is_none())
        {
            *free = Some(ItemStack {
                touched: true,
                ..stack
            });
        }
        true
    }

    /// Walks a unit over to a ground item and takes it into the first free
    /// bag slot.
    fn take_from_ground(&mut self, unit: Entity, loot: Entity) -> bool {
        let Some(Loot(stack)) = self.loot.get(loot).copied() else {
            return true;
        };
        let Some(spot) = self.transform.get(loot).map(|t| t.pos) else {
            return true;
        };
        let Some(from) = self.transform.get(unit).map(|t| t.pos) else {
            return true;
        };
        if !from.within(spot, rules::units(rules::TAKE_ITEM_RANGE)) {
            self.set_order(unit, UnitOrder::Move { pos: spot });
            return false;
        }
        let Some(bag) = self.inventory.get_mut(unit) else {
            return true;
        };
        let Some(free) = bag.slots.iter_mut().find(|held| held.is_none()) else {
            return true;
        };
        *free = Some(ItemStack {
            touched: true,
            ..stack
        });
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
                facing: bota_proto::Angle::default(),
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
    fn held_in_bag(&self, unit: Entity, slot: usize) -> Option<ItemStack> {
        self.inventory
            .get(unit)
            .and_then(|bag| bag.slots.get(slot).copied().flatten())
    }

    /// Takes the stack out of one of a unit's own bag slots.
    fn take_from_bag(&mut self, unit: Entity, slot: usize) -> Option<ItemStack> {
        self.inventory.get_mut(unit)?.slots.get_mut(slot)?.take()
    }
}
