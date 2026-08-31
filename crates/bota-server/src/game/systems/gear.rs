//! Items carried and abilities held: what they add, and what they cost.

use bota_proto::{AbilityId, Attribute, EventKind, Fixed, ItemId, SlotId, Target, Vec2};

use crate::game::{
    AbilityBook, AbilityState, BAG_SLOTS, Carried, Entity, Inventory, ItemStack, ItemUse, Pool,
    Status, StatusKind, World, hero_def, in_backpack, in_inventory, in_stash, item_def, wire_id,
};
use crate::game::{Event, EventVisibility, rules};
use crate::game::{clamp_to_map, move_towards};

/// What one drink of an item does, gathered so it travels as one thing.
struct Mend {
    /// Which pool it mends.
    pool: Pool,
    /// How much it mends over the whole of it.
    total: i32,
    /// How long it runs.
    ticks: u32,
    /// How far it reaches, in world units.
    range: i32,
    /// Whether it takes a tree down to work.
    eats_a_tree: bool,
    /// Whether a blow puts it out.
    breaks: bool,
}

/// How many charges one use costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spends {
    /// None at all.
    Nothing,
    /// One of them.
    One,
    /// Every one it holds.
    Everything,
}

/// Which of the slots held cover a list of parts, one slot to each part.
///
/// Nothing at all when any part is missing: a build takes every part or none.
fn parts_in(held: &[(usize, ItemStack)], parts: &[ItemId]) -> Option<Vec<usize>> {
    let mut spent: Vec<usize> = Vec::with_capacity(parts.len());
    for part in parts {
        let at = held
            .iter()
            .find(|(at, stack)| stack.id == *part && !spent.contains(at))
            .map(|(at, _)| *at)?;
        spent.push(at);
    }
    Some(spent)
}

/// The slots a hero of this kind carries, all unlearned.
pub fn hero_kit(hero: bota_proto::HeroId) -> AbilityBook {
    let carried = hero_def(hero).map_or(&[] as &[AbilityId], |def| def.abilities);
    AbilityBook {
        slots: carried
            .iter()
            .map(|id| AbilityState {
                id: *id,
                level: 0,
                cooldown: 0,
            })
            .collect(),
    }
}

/// What a whole inventory adds.
///
/// Only what sits in the inventory proper counts; what is in the backpack is
/// carried inert.
pub fn carried_bonus(inventory: &Inventory) -> Carried {
    let mut total = Carried::default();
    for stack in inventory
        .slots
        .iter()
        .take(rules::INVENTORY_SLOTS)
        .flatten()
    {
        if stack.mute > 0 {
            continue;
        }
        let Some(def) = item_def(stack.id) else {
            continue;
        };
        total.attributes += def.carried.attributes;
        total.move_speed += def.carried.move_speed;
        total.damage += def.carried.damage;
        total.attack_speed += def.carried.attack_speed;
        total.armor += def.carried.armor;
        total.hp += def.carried.hp;
        total.mana += def.carried.mana;
        total.hp_regen += def.carried.hp_regen;
        total.mana_regen += def.carried.mana_regen;
        total.damage_to_creeps += def.carried.damage_to_creeps;
        // What an item is set to is worth points of that attribute alone.
        if let Some(mode) = stack.mode {
            let bonus = Fixed::from_int(def.mode_bonus);
            match mode {
                Attribute::Strength => total.attributes.strength += bonus,
                Attribute::Agility => total.attributes.agility += bonus,
                Attribute::Intelligence => total.attributes.intelligence += bonus,
            }
        }
    }
    total
}

impl World {
    /// Runs down every cooldown an entity is waiting on.
    pub fn tick_gear(&mut self) {
        // A wait runs whether the body it belongs to is standing or not: what
        // waits with a seat runs down beside what waits on a body.
        for seat in self.seats.iter_mut() {
            for (_, left) in seat.item_clocks.iter_mut() {
                *left = left.saturating_sub(1);
            }
            seat.item_clocks.retain(|(_, left)| *left > 0);
            let Some(kept) = seat.kept.as_mut() else {
                continue;
            };
            for slot in kept.book.slots.iter_mut() {
                slot.cooldown = slot.cooldown.saturating_sub(1);
            }
            for stack in kept.bag.slots.iter_mut().flatten() {
                stack.cooldown = stack.cooldown.saturating_sub(1);
                stack.mute = stack.mute.saturating_sub(1);
            }
        }
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            if let Some(book) = self.abilities.get_mut(entity) {
                for slot in book.slots.iter_mut() {
                    slot.cooldown = slot.cooldown.saturating_sub(1);
                }
            }
            if let Some(bag) = self.inventory.get_mut(entity) {
                for stack in bag.slots.iter_mut().flatten() {
                    stack.cooldown = stack.cooldown.saturating_sub(1);
                    stack.mute = stack.mute.saturating_sub(1);
                }
            }
            if let Some(on_it) = self.statuses.get_mut(entity) {
                for status in on_it.0.iter_mut() {
                    status.ticks_left = status.ticks_left.saturating_sub(1);
                }
                on_it.0.retain(|status| status.ticks_left > 0);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Builds whatever each hero now holds the parts of.
    ///
    /// Run every tick, so a build follows however the last part arrived: a
    /// purchase, a slot moved, or a courier setting one down.
    pub fn assemble_bags(&mut self) {
        for (slot, unit) in self
            .seats
            .iter()
            .filter_map(|seat| seat.unit.map(|unit| (seat.slot, unit)))
            .collect::<Vec<_>>()
        {
            // One build may hand the next its last part, and each one leaves
            // fewer stacks than it took, so the run ends.
            for _ in 0..BAG_SLOTS {
                if !self.assemble_once(slot, unit) {
                    break;
                }
            }
        }
    }

    /// Builds the first item in the catalog whose parts an entity all holds.
    ///
    /// Only what a unit carries itself takes part: the stash is at the shop
    /// and builds nothing. A part somebody else bought and a part marked for
    /// sale take no part either. The build lands in the lowest slot any of
    /// its parts came out of, so it stays in the inventory when a part was
    /// there.
    fn assemble_once(&mut self, slot: SlotId, unit: Entity) -> bool {
        let Some(bag) = self.inventory.get(unit) else {
            return false;
        };
        let held: Vec<(usize, ItemStack)> = bag
            .slots
            .iter()
            .enumerate()
            .take(BAG_SLOTS)
            .filter_map(|(at, held)| held.map(|stack| (at, stack)))
            .filter(|(_, stack)| stack.owner == slot && !stack.for_sale)
            .collect();
        for (index, def) in crate::game::ITEMS.iter().enumerate() {
            if def.components.is_empty() {
                continue;
            }
            let Some(spent) = parts_in(&held, def.components) else {
                continue;
            };
            let landing = spent.iter().copied().min().expect("a build has parts");
            // A build that holds charges takes over whatever its parts held.
            let charges = held
                .iter()
                .filter(|(at, _)| spent.contains(at))
                .filter(|(_, stack)| item_def(stack.id).is_some_and(|part| part.cast_charges > 0))
                .map(|(_, stack)| stack.charges)
                .sum::<u8>()
                .min(def.cast_charges);
            let built = ItemStack {
                id: ItemId(index as u16),
                charges: if def.cast_charges > 0 {
                    charges
                } else {
                    def.charges
                },
                cooldown: 0,
                mute: 0,
                mode: def.mode,
                bought_tick: self.tick,
                touched: true,
                owner: slot,
                for_sale: false,
            };
            let Some(bag) = self.inventory.get_mut(unit) else {
                return false;
            };
            for at in spent {
                bag.slots[at] = None;
            }
            bag.slots[landing] = Some(built);
            return true;
        }
        false
    }

    /// Buys an item for a seat that can afford it.
    ///
    /// At its own shop it goes to the first free slot of the hero's bag, and
    /// to the stash when that bag is full. Anywhere else it goes to the stash
    /// and waits there.
    pub fn buy(&mut self, slot: SlotId, item: ItemId, events: &mut Vec<Event>) -> bool {
        let Some(index) = self.seats.iter().position(|s| s.slot == slot) else {
            return false;
        };
        let Some(unit) = self.seats[index].unit else {
            return false;
        };
        if item_def(item).is_none() {
            return false;
        }
        let parts = self.missing_parts(index, unit, item);
        let price: i32 = parts
            .iter()
            .filter_map(|part| item_def(*part))
            .map(|def| def.cost)
            .sum();
        if self.seats[index].gold < price || self.free_slots(index, unit) < parts.len() {
            return false;
        }
        for part in parts {
            let Some(def) = item_def(part) else {
                continue;
            };
            let bought = ItemStack {
                id: part,
                charges: def.charges,
                cooldown: 0,
                mute: 0,
                mode: def.mode,
                bought_tick: self.tick,
                touched: false,
                owner: slot,
                for_sale: false,
            };
            let in_hand = self.at_shop(unit)
                && self
                    .inventory
                    .get_mut(unit)
                    .and_then(|bag| bag.slots.iter_mut().find(|slot| slot.is_none()))
                    .map(|free| *free = Some(bought))
                    .is_some();
            if !in_hand {
                self.put_in_stash(index, bought);
            }
        }
        self.seats[index].gold -= price;
        events.push(Event {
            kind: EventKind::ItemBought { slot, item },
            visible_to: EventVisibility::OneTeam(self.seats[index].team),
        });
        true
    }

    /// What a seat still has to buy for one item to be had.
    ///
    /// An item bought whole is the whole of the answer, held already or not:
    /// asking for a second one buys a second one. One that is built answers
    /// with the parts it lacks, each of those asked the same question in turn,
    /// so a part that is itself built is bought as its own parts. A part
    /// already waiting in the bag or the stash is spent on the build and so is
    /// bought no second time.
    fn missing_parts(&self, seat: usize, unit: Entity, item: ItemId) -> Vec<ItemId> {
        let mut held: Vec<ItemId> = self
            .inventory
            .get(unit)
            .into_iter()
            .flat_map(|bag| bag.slots.iter().take(BAG_SLOTS).flatten())
            .chain(self.seats[seat].stash.slots.iter().flatten())
            .map(|stack| stack.id)
            .collect();
        let mut wanted = Vec::new();
        // What was asked for is bought however many of it are already held:
        // only its parts are looked for in hand.
        match item_def(item).map_or(&[][..], |def| def.components) {
            [] => wanted.push(item),
            parts => {
                for part in parts {
                    self.parts_beyond(*part, &mut held, &mut wanted);
                }
            }
        }
        wanted
    }

    /// Lays out what one item still costs, spending `held` as it goes.
    fn parts_beyond(&self, item: ItemId, held: &mut Vec<ItemId>, wanted: &mut Vec<ItemId>) {
        if let Some(at) = held.iter().position(|id| *id == item) {
            held.remove(at);
            return;
        }
        let parts = item_def(item).map_or(&[][..], |def| def.components);
        if parts.is_empty() {
            wanted.push(item);
            return;
        }
        for part in parts {
            self.parts_beyond(*part, held, wanted);
        }
    }

    /// Slots a seat has free for something bought right now.
    fn free_slots(&self, seat: usize, unit: Entity) -> usize {
        let stash = self.seats[seat]
            .stash
            .slots
            .iter()
            .filter(|slot| slot.is_none())
            .count();
        if !self.at_shop(unit) {
            return stash;
        }
        let bag = self
            .inventory
            .get(unit)
            .map_or(0, |bag| bag.slots.iter().filter(|s| s.is_none()).count());
        bag + stash
    }

    /// Lays a stack in the first free slot of a seat's stash.
    fn put_in_stash(&mut self, seat: usize, stack: ItemStack) -> bool {
        let Some(free) = self.seats[seat]
            .stash
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
        else {
            return false;
        };
        *free = Some(stack);
        true
    }

    /// Uses what sits in one of an entity's item slots.
    ///
    /// A use that does nothing spends nothing: the charge, the cooldown and
    /// the slot are only touched once whatever the item does has been done.
    pub fn use_item(&mut self, entity: Entity, slot: usize, target: Target) -> bool {
        let Some(bag) = self.inventory.get(entity) else {
            return false;
        };
        let Some(Some(stack)) = bag.slots.get(slot).cloned() else {
            return false;
        };
        if slot >= rules::INVENTORY_SLOTS || stack.cooldown > 0 || stack.mute > 0 {
            return false;
        }
        let Some(def) = item_def(stack.id) else {
            return false;
        };
        if def.shared_wait && self.owes_wait(entity, stack.id) {
            return false;
        }
        if (def.charges > 0 || def.cast_charges > 0) && stack.charges == 0 {
            return false;
        }
        let Some(active) = def.active else {
            return false;
        };
        if def.mana_cost > 0
            && self
                .mana
                .get(entity)
                .is_none_or(|pool| pool.mana < Fixed::from_int(def.mana_cost))
        {
            return false;
        }
        let done = match active {
            ItemUse::Mend {
                pool,
                total,
                ticks,
                range,
                eats_a_tree,
                breaks,
            } => self.mend_with(
                entity,
                target,
                Mend {
                    pool,
                    total,
                    ticks,
                    range,
                    eats_a_tree,
                    breaks,
                },
            ),
            ItemUse::Teleport { channel, range } => {
                self.begin_teleport(entity, target, channel, range, slot)
            }
            ItemUse::Ward { def, ticks, range } => {
                self.stand_ward(entity, target, def, ticks, range)
            }
            ItemUse::Fell { range } => self.fell_a_tree(entity, target, range),
            ItemUse::Plant { ticks, range } => self.plant_a_tree(entity, target, ticks, range),
            ItemUse::Restore {
                hp_per_charge,
                mana_per_charge,
            } => self.restore_with(
                entity,
                i32::from(stack.charges) * hp_per_charge,
                i32::from(stack.charges) * mana_per_charge,
            ),
            ItemUse::Blink { range } => self.blink_to(entity, target, range),
            ItemUse::Phase { pct, ticks } => self.walk_through(entity, pct, ticks),
            ItemUse::Switch => self.switch_mode(entity, slot),
        };
        if !done {
            return false;
        }
        if def.mana_cost > 0
            && let Some(pool) = self.mana.get_mut(entity)
        {
            pool.mana -= Fixed::from_int(def.mana_cost);
        }
        let cooldown = if def.shared_wait { 0 } else { def.cooldown };
        // A scroll is spent when it carries, which is the teleport's own
        // business; everything else is spent the moment it is used.
        let spends = match active {
            ItemUse::Teleport { .. } | ItemUse::Switch => Spends::Nothing,
            ItemUse::Restore { .. } => Spends::Everything,
            _ if def.charges > 0 => Spends::One,
            _ => Spends::Nothing,
        };
        if def.shared_wait {
            self.owe_wait(entity, stack.id, def.cooldown);
        }
        if let Some(bag) = self.inventory.get_mut(entity)
            && let Some(held) = bag.slots.get_mut(slot)
            && let Some(stack) = held
        {
            stack.cooldown = cooldown;
            stack.touched = true;
            stack.charges = match spends {
                Spends::Nothing => stack.charges,
                Spends::One => stack.charges.saturating_sub(1),
                Spends::Everything => 0,
            };
            // What gains charges again is kept when the last one goes;
            // what does not is gone with it.
            if stack.charges == 0 && def.cast_charges == 0 && spends != Spends::Nothing {
                *held = None;
            }
        }
        true
    }

    /// Mends whoever used an item, at once.
    fn restore_with(&mut self, on: Entity, hp: i32, mana: i32) -> bool {
        if hp <= 0 && mana <= 0 {
            return false;
        }
        let ceiling = self.stats.get(on).copied();
        if let Some(pool) = self.health.get_mut(on)
            && let Some(most) = ceiling.map(|stats| stats.max_hp)
        {
            pool.hp = (pool.hp + Fixed::from_int(hp)).min(most);
        }
        if let Some(pool) = self.mana.get_mut(on)
            && let Some(most) = ceiling.map(|stats| stats.max_mana)
        {
            pool.mana = (pool.mana + Fixed::from_int(mana)).min(most);
        }
        true
    }

    /// Walks whoever used an item faster, and through whatever is in the way.
    fn walk_through(&mut self, user: Entity, pct: i32, ticks: u32) -> bool {
        let mut on_it = self.statuses.remove(user).unwrap_or_default();
        on_it.put(Status {
            kind: StatusKind::Hastened { pct },
            ticks_left: ticks,
        });
        on_it.put(Status {
            kind: StatusKind::Phased,
            ticks_left: ticks,
        });
        self.statuses.insert(user, on_it);
        true
    }

    /// Sets what sits in a slot to the attribute after the one it is on.
    fn switch_mode(&mut self, user: Entity, slot: usize) -> bool {
        let Some(bag) = self.inventory.get_mut(user) else {
            return false;
        };
        let Some(Some(stack)) = bag.slots.get_mut(slot) else {
            return false;
        };
        let Some(mode) = stack.mode else {
            return false;
        };
        stack.mode = Some(match mode {
            Attribute::Strength => Attribute::Agility,
            Attribute::Agility => Attribute::Intelligence,
            Attribute::Intelligence => Attribute::Strength,
        });
        true
    }

    /// Whether whoever holds this entity still owes a wait on a kind of item.
    pub fn owes_wait(&self, entity: Entity, item: ItemId) -> bool {
        self.seats
            .iter()
            .find(|seat| seat.unit == Some(entity))
            .is_some_and(|seat| seat.item_clocks.iter().any(|(held, _)| *held == item))
    }

    /// Starts the wait a kind of item owes its user.
    fn owe_wait(&mut self, entity: Entity, item: ItemId, ticks: u32) {
        let Some(seat) = self.seats.iter_mut().find(|seat| seat.unit == Some(entity)) else {
            return;
        };
        seat.item_clocks.retain(|(held, _)| *held != item);
        if ticks > 0 {
            seat.item_clocks.push((item, ticks));
        }
    }

    /// Puts a mending effect on whoever an item was used on.
    ///
    /// It reaches one of its user's own side, standing within `range`. Aimed
    /// at nothing at all, it lands on the one who used it.
    fn mend_with(&mut self, user: Entity, target: Target, drink: Mend) -> bool {
        let Mend {
            pool,
            total,
            ticks,
            range,
            eats_a_tree,
            breaks,
        } = drink;
        let on = match target {
            Target::Unit(target) => match self.of_wire(target) {
                Some(on) => on,
                None => return false,
            },
            Target::None | Target::Pos(..) => user,
        };
        if !self.alive(on) || self.team.get(on) != self.team.get(user) {
            return false;
        }
        let (Some(from), Some(at)) = (
            self.transform.get(user).map(|t| t.pos),
            self.transform.get(on).map(|t| t.pos),
        ) else {
            return false;
        };
        if !from.within(at, rules::units(range)) {
            return false;
        }
        // What eats a tree is paid for by one: with none in reach nothing is
        // eaten and nothing is spent. One that was put up feeds twice as long
        // as one the map grew.
        let mut ticks = ticks;
        let mut total = total;
        if eats_a_tree {
            let Some(tree) = self.reach_a_tree(user, target, range) else {
                return false;
            };
            if matches!(tree, crate::game::Tree::Planted(_)) {
                ticks *= 2;
                total *= 2;
            }
            self.trees.fell(tree, self.tick);
            self.lay_sight_block();
            self.lay_passability();
        }
        let per_tick = total * 100 / ticks.max(1) as i32;
        let put = Status {
            kind: match pool {
                Pool::Health => StatusKind::Mending { per_tick, breaks },
                Pool::Mana => StatusKind::Clarity { per_tick, breaks },
            },
            ticks_left: ticks,
        };
        match self.statuses.get_mut(on) {
            Some(on_it) => on_it.put(put),
            None => {
                let mut on_it = crate::game::Statuses::default();
                on_it.put(put);
                self.statuses.insert(on, on_it);
            }
        }
        true
    }

    /// Burns one charge of what sits in a slot, clearing the slot when the
    /// last one goes.
    pub fn spend_charge(&mut self, entity: Entity, slot: usize) {
        if let Some(bag) = self.inventory.get_mut(entity)
            && let Some(held) = bag.slots.get_mut(slot)
            && let Some(stack) = held
        {
            stack.touched = true;
            stack.charges = stack.charges.saturating_sub(1);
            if stack.charges == 0 {
                *held = None;
            }
        }
    }

    /// Moves what sits in one slot of a unit to another, swapping whatever is
    /// in the way.
    ///
    /// Slots run the unit's own bag first, then the seat's stash. The stash
    /// takes part only while that unit stands in its own shop, so a courier
    /// waiting at the fountain reaches it as readily as a hero does. A stack coming out of the
    /// backpack into the inventory is muted for a while.
    pub fn move_item(&mut self, slot: SlotId, unit: Entity, from: usize, to: usize) -> bool {
        let Some(seat) = self.seats.iter().position(|s| s.slot == slot) else {
            return false;
        };
        let total = BAG_SLOTS + self.seats[seat].stash.slots.len();
        if from == to || from >= total || to >= total {
            return false;
        }
        if (in_stash(from) || in_stash(to)) && !self.at_shop(unit) {
            return false;
        }
        let Some(moved) = self.take_slot(unit, seat, from) else {
            return false;
        };
        let displaced = self.take_slot(unit, seat, to);
        self.put_slot(unit, seat, to, Some(moved));
        self.put_slot(unit, seat, from, displaced);
        for (origin, landed) in [(from, to), (to, from)] {
            let mute = in_backpack(origin) && in_inventory(landed);
            if let Some(stack) = self.slot_mut(unit, seat, landed) {
                stack.touched = true;
                if mute {
                    stack.mute = stack.mute.max(rules::BACKPACK_MUTE_TICKS);
                }
            }
        }
        true
    }

    /// Sells what sits in one slot, or marks it to be sold.
    ///
    /// The stash sells from wherever its owner is: it is already at the shop.
    /// A unit's own bag sells only while that unit stands there; anywhere
    /// else the order marks the stack for sale, and a second order on the
    /// slot unmarks it. Only the seat that bought a stack may sell or mark
    /// it.
    ///
    /// An untouched stack sold soon after it was bought pays back what it
    /// cost; anything else pays back a part of it.
    pub fn sell_item(&mut self, slot: SlotId, unit: Entity, at: usize) -> bool {
        let Some(seat) = self.seats.iter().position(|s| s.slot == slot) else {
            return false;
        };
        let held = if in_stash(at) {
            self.seats[seat]
                .stash
                .slots
                .get(at - BAG_SLOTS)
                .copied()
                .flatten()
        } else {
            self.inventory
                .get(unit)
                .and_then(|bag| bag.slots.get(at).copied().flatten())
        };
        let Some(held) = held else {
            return false;
        };
        if held.owner != slot {
            return false;
        }
        // What waits in the stash waits at the shop, so it sells from
        // anywhere; what a unit carries sells only where that unit stands.
        // Anywhere else the order is a mark, and marks toggle.
        if !in_stash(at) && !self.at_shop(unit) {
            if let Some(stack) = self.slot_mut(unit, seat, at) {
                stack.for_sale = !stack.for_sale;
                return true;
            }
            return false;
        }
        let Some(stack) = self.take_slot(unit, seat, at) else {
            return false;
        };
        if item_def(stack.id).is_none() {
            self.put_slot(unit, seat, at, Some(stack));
            return false;
        }
        let back = self.sale_price(&stack);
        self.seats[seat].gold += back;
        true
    }

    /// What selling a stack pays right now.
    fn sale_price(&self, stack: &ItemStack) -> i32 {
        let Some(def) = item_def(stack.id) else {
            return 0;
        };
        let fresh = !stack.touched
            && self.tick.saturating_sub(stack.bought_tick) <= rules::SELL_REFUND_TICKS;
        if fresh {
            def.cost
        } else {
            def.cost * rules::SELL_PCT / 100
        }
    }

    /// Sells every stack marked for sale that has reached the shop.
    ///
    /// A marked stack sells wherever it sits — its owner's stash, or any bag
    /// standing within the owner's home shop area — and the gold goes to the
    /// owner. One pass owns every way a stack can arrive: carried there,
    /// delivered by courier, or put back into the stash.
    pub fn settle_sales(&mut self) {
        for seat in 0..self.seats.len() {
            for at in 0..self.seats[seat].stash.slots.len() {
                let Some(stack) = self.seats[seat].stash.slots[at] else {
                    continue;
                };
                if !stack.for_sale {
                    continue;
                }
                let back = self.sale_price(&stack);
                self.seats[seat].stash.slots[at] = None;
                if let Some(owner) = self.seats.iter().position(|s| s.slot == stack.owner) {
                    self.seats[owner].gold += back;
                }
            }
        }
        let entities = self.take_entity_snapshot();
        for holder in entities.iter().copied() {
            let Some(at_pos) = self.transform.get(holder).map(|t| t.pos) else {
                continue;
            };
            let Some(bag) = self.inventory.get(holder) else {
                continue;
            };
            let marked: Vec<(usize, ItemStack)> = bag
                .slots
                .iter()
                .enumerate()
                .filter_map(|(at, held)| held.filter(|stack| stack.for_sale).map(|s| (at, s)))
                .collect();
            for (at, stack) in marked {
                let Some(owner) = self.seats.iter().position(|s| s.slot == stack.owner) else {
                    continue;
                };
                let shop = crate::game::fountain_pos(self.map, self.seats[owner].team);
                if !at_pos.within(shop, rules::units(rules::SHOP_RANGE)) {
                    continue;
                }
                let back = self.sale_price(&stack);
                if let Some(bag) = self.inventory.get_mut(holder)
                    && let Some(held) = bag.slots.get_mut(at)
                {
                    *held = None;
                }
                self.seats[owner].gold += back;
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Takes whatever sits in one slot of a seat's slots out of it.
    fn take_slot(&mut self, unit: Entity, seat: usize, slot: usize) -> Option<ItemStack> {
        if in_stash(slot) {
            self.seats[seat]
                .stash
                .slots
                .get_mut(slot - BAG_SLOTS)?
                .take()
        } else {
            self.inventory.get_mut(unit)?.slots.get_mut(slot)?.take()
        }
    }

    /// Lays a stack in one slot, over whatever was there.
    fn put_slot(&mut self, unit: Entity, seat: usize, slot: usize, stack: Option<ItemStack>) {
        if let Some(held) = self.slot_of(unit, seat, slot) {
            *held = stack;
        }
    }

    /// The stack in one slot, to be changed in place.
    fn slot_mut(&mut self, unit: Entity, seat: usize, slot: usize) -> Option<&mut ItemStack> {
        self.slot_of(unit, seat, slot)?.as_mut()
    }

    /// One slot of a seat's slots, wherever it lives.
    fn slot_of(
        &mut self,
        unit: Entity,
        seat: usize,
        slot: usize,
    ) -> Option<&mut Option<ItemStack>> {
        if in_stash(slot) {
            self.seats[seat].stash.slots.get_mut(slot - BAG_SLOTS)
        } else {
            self.inventory.get_mut(unit)?.slots.get_mut(slot)
        }
    }

    /// Carries whoever used an item to a point.
    ///
    /// Aimed further off than it carries, it carries as far as it does along
    /// the same line. A landing spot on closed ground steps back along that
    /// line until it finds open ground, and the blink fails when it finds
    /// none.
    fn blink_to(&mut self, user: Entity, target: Target, range: i32) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let Some(from) = self.transform.get(user).map(|t| t.pos) else {
            return false;
        };
        let reach = rules::units(range);
        let aim = clamp_to_map(if from.within(pos, reach) {
            pos
        } else {
            move_towards(from, pos, reach)
        });
        let Some(landing) = self.open_ground_back(from, aim) else {
            return false;
        };
        if let Some(transform) = self.transform.get_mut(user) {
            transform.pos = landing;
        }
        true
    }

    /// The spot nearest `aim` on the line back to `from` that is open ground.
    fn open_ground_back(&self, from: Vec2, aim: Vec2) -> Option<Vec2> {
        let step = rules::units(rules::BLINK_STEP_BACK);
        let mut at = aim;
        for _ in 0..rules::BLINK_STEP_TRIES {
            if self.grid.walkable(at) {
                return Some(at);
            }
            if at == from {
                break;
            }
            at = move_towards(at, from, step);
        }
        None
    }

    /// The tree an item was aimed at.
    ///
    /// The spot aimed at has to fall inside a tree's own circle: what answers
    /// is the tree that was pointed at, not whatever tree happened to be
    /// nearest. That tree then has to be within reach of the one using the
    /// item.
    fn reach_a_tree(&self, user: Entity, target: Target, range: i32) -> Option<crate::game::Tree> {
        let from = self.transform.get(user).map(|t| t.pos)?;
        let at = match target {
            Target::Pos(pos) => pos,
            Target::Unit(target) => self.transform.get(self.of_wire(target)?)?.pos,
            Target::None => from,
        };
        let tree = self
            .trees
            .nearest(self.map, at, rules::units(rules::TREE_RADIUS))?;
        let spot = self.trees.spot(self.map, tree)?;
        from.within(spot, rules::units(range)).then_some(tree)
    }

    /// Takes down the tree an item was aimed at.
    fn fell_a_tree(&mut self, user: Entity, target: Target, range: i32) -> bool {
        let Some(tree) = self.reach_a_tree(user, target, range) else {
            return false;
        };
        self.trees.fell(tree, self.tick);
        self.lay_sight_block();
        self.lay_passability();
        true
    }

    /// Puts a tree up where an item was aimed, on ground that has none.
    fn plant_a_tree(&mut self, user: Entity, target: Target, ticks: u32, range: i32) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let Some(from) = self.transform.get(user).map(|t| t.pos) else {
            return false;
        };
        if !from.within(pos, rules::units(range)) || !self.grid.walkable(pos) {
            return false;
        }
        // Not on top of a tree already standing there.
        if self
            .trees
            .nearest(self.map, pos, rules::units(rules::TREE_RADIUS))
            .is_some()
        {
            return false;
        }
        let until = self.tick + ticks;
        self.trees.plant(pos, until);
        self.lay_sight_block();
        self.lay_passability();
        true
    }

    /// Puts an ability point into one slot of an entity's book.
    ///
    /// A point has to be there to spend: a hero has one per level and no more.
    /// Each level of an ability waits for a hero level of its own, and none
    /// goes past its own cap. A point into a slot that shares its level with
    /// others — a raze — levels every slot of the group at once.
    pub fn learn(&mut self, entity: Entity, slot: usize, events: &mut Vec<Event>) -> bool {
        let Some(book) = self.abilities.get(entity) else {
            return false;
        };
        let Some(ability) = book.slots.get(slot).copied() else {
            return false;
        };
        let Some(def) = crate::game::ability_def(ability.id) else {
            return false;
        };
        if ability.level >= def.max_level {
            return false;
        }
        let hero_level = self.level.get(entity).map_or(1, |level| level.0);
        if self.points_spent(entity) >= hero_level
            || hero_level < crate::game::level_floor(def, ability.level)
        {
            return false;
        }
        let group = crate::game::learn_group(ability.id);
        let Some(book) = self.abilities.get_mut(entity) else {
            return false;
        };
        for held in &mut book.slots {
            if crate::game::learn_group(held.id) == group {
                held.level += 1;
            }
        }
        events.push(Event {
            kind: EventKind::AbilityCast {
                caster: wire_id(entity),
                ability: ability.id,
            },
            visible_to: EventVisibility::Everyone,
        });
        true
    }

    /// Skill points an entity has spent.
    ///
    /// Slots sharing a learn group were paid for together, so the group
    /// counts once.
    pub fn points_spent(&self, entity: Entity) -> u8 {
        let Some(book) = self.abilities.get(entity) else {
            return 0;
        };
        book.slots
            .iter()
            .enumerate()
            .filter(|(at, held)| {
                let group = crate::game::learn_group(held.id);
                !book.slots[..*at]
                    .iter()
                    .any(|other| crate::game::learn_group(other.id) == group)
            })
            .map(|(_, held)| held.level)
            .sum()
    }
}
