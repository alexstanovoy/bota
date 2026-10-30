//! Item errands: walking an item out of a bag, into another, or off the
//! ground.

use bota_core::rules;
use bota_proto::{Angle, EntityId, EventKind, Fixed, Target, Team, Vec2};

use crate::game::{
    BAG_SLOTS, Entity, Event, EventVisibility, Handling, ItemStack, Loot, Transform, UnitOrder,
    Visibility, World, wire_id,
};

impl World {
    /// Starts walking an item out of one of a unit's bag slots.
    ///
    /// Aimed at a point the item will be laid there; aimed at nothing, at the
    /// unit's own feet; aimed at an allied unit with a bag, handed into it.
    /// The walking and the doing live with [`World::tick_handling`].
    pub fn put_item(&mut self, unit: Entity, slot: usize, target: Target) -> bool {
        if slot >= BAG_SLOTS
            || self
                .held_in_bag(unit, slot)
                .is_none_or(|stack| stack.rune.is_some())
        {
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

    /// Starts walking a hero over to a rune to destroy it: an attack order
    /// at a rune [LIQ Runes: "Can be destroyed by an attack command"].
    pub fn deny_rune_order(&mut self, unit: Entity, target: EntityId) -> bool {
        let Some(mark) = self.of_wire(target) else {
            return false;
        };
        if self.rune.get(mark).is_none() || !self.is_real_hero(unit) {
            return false;
        }
        self.handling.insert(unit, Handling::Deny { rune: mark });
        true
    }

    /// Starts walking a unit over to a ground item to take it.
    pub fn take_item(&mut self, unit: Entity, target: EntityId) -> bool {
        let Some(mark) = self.of_wire(target) else {
            return false;
        };
        // A rune is walked to, and a hero within reach of it takes it
        // ([`World::take_runes`]); a ground item goes into a free slot.
        let takes = if self.rune.get(mark).is_some() {
            self.is_real_hero(unit)
        } else {
            self.loot.get(mark).is_some() && self.inventory.get(unit).is_some()
        };
        if !takes {
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
                Handling::Deny { rune } => self.deny_rune(unit, rune),
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
        if self.pos(unit).is_none() {
            return true;
        }
        if !self.walk_into_reach(unit, pos, rules::PUT_ITEM_RANGE) {
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
        let (Some(_), Some(at)) = (self.pos(unit), self.pos(to)) else {
            return true;
        };
        if !self.walk_into_reach(unit, at, rules::PUT_ITEM_RANGE) {
            return false;
        }
        let Some(stack) = self.take_from_bag(unit, slot) else {
            return true;
        };
        let handed = self.as_received_by(
            to,
            ItemStack {
                touched: true,
                ..stack
            },
        );
        let taken = self
            .inventory
            .get_mut(to)
            .is_some_and(|bag| bag.receive(handed));
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
        if self.rune.get(loot).is_some() {
            return self.walk_to_rune(unit, loot);
        }
        let Some(Loot(stack)) = self.loot.get(loot).copied() else {
            return true;
        };
        let Some(spot) = self.pos(loot) else {
            return true;
        };
        if self.pos(unit).is_none() {
            return true;
        }
        if !self.walk_into_reach(unit, spot, rules::TAKE_ITEM_RANGE) {
            return false;
        }
        let received = self.as_received_by(
            unit,
            ItemStack {
                touched: true,
                ..stack
            },
        );
        let taken = self
            .inventory
            .get_mut(unit)
            .is_some_and(|bag| bag.receive(received));
        if !taken {
            return true;
        }
        self.loot.remove(loot);
        self.despawn(loot);
        true
    }

    /// A stack as it is once in the bag of `unit`: a build that enters the bag of a
    /// unit its owner does not drive can no longer be taken apart (Dota ends the
    /// window there).
    fn as_received_by(&self, unit: Entity, stack: ItemStack) -> ItemStack {
        if self.owner.get(unit) == Some(&stack.owner) {
            return stack;
        }
        ItemStack {
            disassemble_until: 0,
            ..stack
        }
    }

    /// Walks a hero into attack range of a rune and destroys it there. An
    /// attack on a rune is neither a last hit nor a deny; both sides are
    /// told. The swing's own time is not spent.
    fn deny_rune(&mut self, unit: Entity, rune: Entity) -> bool {
        let (Some(held), Some(spot), Some(from)) =
            (self.rune.get(rune).copied(), self.pos(rune), self.pos(unit))
        else {
            return true;
        };
        let reach = self
            .stats
            .get(unit)
            .map_or(Fixed::ZERO, |stats| stats.attack_range);
        if !from.within(spot, reach) {
            self.set_order(unit, UnitOrder::Move { pos: spot });
            return false;
        }
        self.rune.remove(rune);
        self.despawn(rune);
        self.events.push(Event {
            kind: EventKind::RuneDenied {
                unit: wire_id(unit),
                kind: held.kind,
                at: spot,
                team: self.team.get(unit).copied().unwrap_or(Team::Neutral),
            },
            visible_to: EventVisibility::Everyone,
            gold_hidden_from: None,
            source_told_to: EventVisibility::Everyone,
        });
        true
    }

    /// Walks a hero over to a rune until it is within reach: the rune is
    /// taken the tick the hero is, by [`World::take_runes`]. An errand ends
    /// with the rune, taken by anyone or gone with the next spawn.
    fn walk_to_rune(&mut self, unit: Entity, rune: Entity) -> bool {
        let Some(spot) = self.pos(rune) else {
            return true;
        };
        if self.pos(unit).is_none() {
            return true;
        }
        self.walk_into_reach(unit, spot, bota_core::RUNE_PICKUP_RANGE)
    }

    /// Whether a unit stands within `range` of a spot, sending it walking
    /// there when it does not.
    pub(super) fn walk_into_reach(&mut self, unit: Entity, to: Vec2, range: i32) -> bool {
        if self
            .pos(unit)
            .is_some_and(|at| at.within(to, rules::units(range)))
        {
            return true;
        }
        self.set_order(unit, UnitOrder::Move { pos: to });
        false
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

    pub(crate) fn take_from_bag(&mut self, unit: Entity, slot: usize) -> Option<ItemStack> {
        self.inventory.get_mut(unit)?.slot_mut(slot)?.take()
    }
}
