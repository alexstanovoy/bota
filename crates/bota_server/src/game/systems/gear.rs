//! Items carried and abilities held: what they add, and what they cost.

use bota_core::{
    Carried, CarriedItem, ITEMS, ItemDef, Mend, ModifierKind, Pool, Spends, clamp_to_map,
    effect_def, fountain_pos, hero_def, item_def, learn_group, move_towards, rules,
};
use bota_proto::{
    AbilityId, Attribute, EventKind, Fixed, HeroId, Inflictor, ItemId, SlotId, Target, Vec2,
};

use crate::game::{
    AbilityBook, AbilityState, BAG_SLOTS, Entity, Event, EventVisibility, Inventory, ItemStack,
    Modifier, Tree, World, in_backpack, in_inventory, in_stash, in_town_portal, wire_id,
};

/// Missing purchase parts, their total price, and destination capacity.
pub(crate) struct PurchasePlan {
    parts: Vec<ItemId>,
    pub(crate) cost: i32,
    pub(crate) fits: bool,
}

/// The slots held that cover a list of parts, one slot to each part.
#[derive(Clone, Copy)]
struct Spent {
    slots: [usize; MAX_COMPONENTS],
    len: usize,
}

impl Spent {
    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.slots[..self.len].iter().copied()
    }

    fn contains(&self, slot: usize) -> bool {
        self.slots[..self.len].contains(&slot)
    }

    fn min(&self) -> Option<usize> {
        self.iter().min()
    }
}

/// The most parts any build in the catalog asks for.
const MAX_COMPONENTS: usize = 4;

/// Which of the slots held cover a list of parts, one slot to each part.
///
/// Nothing at all when any part is missing: a build takes every part or none.
fn parts_in(held: &[(usize, ItemStack)], parts: &[ItemId]) -> Option<Spent> {
    assert!(parts.len() <= MAX_COMPONENTS);
    let mut spent = Spent {
        slots: [0; MAX_COMPONENTS],
        len: 0,
    };
    for part in parts {
        let at = held
            .iter()
            .find(|(at, stack)| stack.id == *part && !spent.contains(*at))
            .map(|(at, _)| *at)?;
        spent.slots[spent.len] = at;
        spent.len += 1;
    }
    Some(spent)
}

/// The slots a hero of this kind carries, all unlearned.
pub fn hero_kit(hero: HeroId) -> AbilityBook {
    let carried = hero_def(hero).map_or(&[] as &[AbilityId], |def| def.abilities);
    AbilityBook {
        tree: hero_def(hero).and_then(|def| def.talents),
        talents: bota_proto::Talents::default(),
        attribute_bonus: 0,
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
/// carried inert, and so is a muted stack. Of the boots carried only the
/// fastest counts, and of the laces only the fastest, the two added up. A
/// `ranged` carrier wears what an item adds to one.
pub fn carried_bonus(inventory: &Inventory, ranged: bool) -> Carried {
    bota_core::carried_bonus(
        inventory
            .slots
            .iter()
            .take(rules::INVENTORY_SLOTS)
            .flatten()
            .filter(|stack| stack.mute == 0)
            .map(|stack| CarriedItem {
                id: stack.id,
                charges: stack.charges,
                mode: stack.mode,
            }),
        ranged,
    )
}

impl World {
    /// Runs down every ability and item cooldown and every mute, on bodies
    /// and on what a fallen hero's seat keeps, every seat's item waits, and
    /// every timed modifier, dropping the modifiers and waits that run out.
    pub fn tick_gear(&mut self) {
        // A wait runs whether the body it belongs to is standing or not: what
        // waits with a seat runs down beside what waits on a body.
        for seat in self.seats.iter_mut() {
            seat.item_clocks.retain_mut(|(_, left)| {
                *left = left.saturating_sub(1);
                *left > 0
            });
            let Some(kept) = seat.kept.as_mut() else {
                continue;
            };
            for slot in kept.book.slots.iter_mut() {
                slot.cooldown = slot.cooldown.saturating_sub(1);
            }
            for stack in kept.bag.every_stack_mut() {
                stack.cooldown = stack.cooldown.saturating_sub(1);
                stack.mute = stack.mute.saturating_sub(1);
            }
        }
        for entity in self.entities.iter() {
            if let Some(book) = self.abilities.get_mut(entity) {
                for slot in book.slots.iter_mut() {
                    slot.cooldown = slot.cooldown.saturating_sub(1);
                }
            }
            if let Some(bag) = self.inventory.get_mut(entity) {
                for stack in bag.every_stack_mut() {
                    stack.cooldown = stack.cooldown.saturating_sub(1);
                    stack.mute = stack.mute.saturating_sub(1);
                }
            }
            if let Some(on_it) = self.modifiers.get_mut(entity) {
                let mut lapsed = false;
                on_it.0.retain_mut(|held| {
                    held.ticks_left = held.ticks_left.map(|left| left.saturating_sub(1));
                    let over = held.ticks_left == Some(0);
                    lapsed |= over
                        && effect_def(held.kind.effect_id()).is_some_and(|def| def.ends_its_wearer);
                    !over
                });
                if lapsed {
                    self.lapsed_lives.push(entity);
                }
            }
        }
        self.tick_teleports();
    }

    /// Builds whatever each hero now holds the parts of.
    ///
    /// Run every tick, so a build follows however the last part arrived: a
    /// purchase, a slot moved, or a courier setting one down. The wards of a
    /// hero and of its courier come together into a Ward Dispenser here.
    pub fn assemble_bags(&mut self) {
        for seat in 0..self.seats.len() {
            for carrier in [self.seats[seat].unit, self.seats[seat].courier]
                .into_iter()
                .flatten()
            {
                if let Some(bag) = self.inventory.get_mut(carrier) {
                    bag.combine_wards();
                }
            }
            let (slot, Some(unit)) = (self.seats[seat].slot, self.seats[seat].unit) else {
                continue;
            };
            // One build may hand the next its last part, and each one leaves
            // fewer stacks than it took, so the run ends.
            for _ in 0..BAG_SLOTS {
                if !self.assemble_once(slot, unit) {
                    break;
                }
            }
        }
    }

    /// Builds the first item in the catalog whose parts an entity all holds,
    /// of the first of its recipes they cover, set to that recipe's
    /// attribute.
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
        for (index, def) in ITEMS.iter().enumerate() {
            let Some((spent, mode)) = def
                .recipes()
                .find_map(|recipe| parts_in(&held, recipe.parts).map(|spent| (spent, recipe.mode)))
            else {
                continue;
            };
            let landing = spent.min().expect("a build has parts");
            // A build that holds charges takes over whatever its parts held.
            let charges = held
                .iter()
                .filter(|(at, _)| spent.contains(*at))
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
                mode,
                bought_tick: self.tick,
                touched: true,
                owner: slot,
                for_sale: false,
                free_charges: 0,
                sentries: 0,
                sentry_active: false,
            };
            let Some(bag) = self.inventory.get_mut(unit) else {
                return false;
            };
            for at in spent.iter() {
                bag.slots[at] = None;
            }
            bag.slots[landing] = Some(built);
            return true;
        }
        false
    }

    /// Buys an item for a seat that can afford it.
    ///
    /// Mergeable charges fill a compatible stack before taking an empty slot.
    /// At its own shop it goes to the hero's bag, and
    /// to the stash when that bag is full. Anywhere else, and while the hero
    /// lies dead, it goes to the stash and waits there (Dota: what is bought
    /// away from the fountain goes to the stash).
    pub fn buy(&mut self, slot: SlotId, item: ItemId, events: &mut Vec<Event>) -> bool {
        let Some(index) = self.seats.iter().position(|s| s.slot == slot) else {
            return false;
        };
        let unit = self.seats[index].unit;
        if !self.in_stock(self.seats[index].team, item)
            || item_def(item).is_none_or(|d| d.unbuyable)
        {
            return false;
        }
        if item_def(item).is_some_and(|def| def.stack_limit > 0) {
            return self.buy_stackable(index, unit, item, events);
        }
        let Some(plan) = self.purchase_plan(slot, item) else {
            return false;
        };
        if self.seats[index].gold < plan.cost || !plan.fits {
            return false;
        }
        let at_home = self.at_home(unit);
        for part in plan.parts {
            let bought = ItemStack::bought(part, slot, self.tick).expect("known catalog component");
            let in_hand = at_home
                && unit
                    .and_then(|unit| self.inventory.get_mut(unit))
                    .and_then(|bag| bag.slots.iter_mut().find(|slot| slot.is_none()))
                    .map(|free| *free = Some(bought))
                    .is_some();
            if !in_hand {
                assert!(self.seats[index].stash.receive(bought));
            }
        }
        self.finish_purchase(index, item, plan.cost, events);
        true
    }

    /// Whether a purchase fits at its current destination, independent of affordability.
    pub fn purchase_fits(&self, slot: SlotId, item: ItemId) -> bool {
        self.purchase_plan(slot, item).is_some_and(|plan| plan.fits)
    }

    /// Plans a known item's purchase into the seat hero's bag or stash: the
    /// stash alone while the hero lies dead.
    pub(crate) fn purchase_plan(&self, slot: SlotId, item: ItemId) -> Option<PurchasePlan> {
        let seat = self.seats.iter().position(|seat| seat.slot == slot)?;
        let unit = self.seats[seat].unit;
        let def = item_def(item)?;
        let parts = self.missing_parts(seat, unit, item);
        let cost = parts
            .iter()
            .map(|part| item_def(*part).expect("known catalog component").cost)
            .sum();
        let fits = if def.stack_limit == 0 {
            self.free_slots(seat, unit) >= parts.len()
        } else {
            assert!(def.components.is_empty());
            let bought = ItemStack::bought(item, slot, self.tick).expect("known catalog item");
            (self.at_home(unit)
                && unit
                    .and_then(|unit| self.inventory.get(unit))
                    .is_some_and(|bag| bag.receiving_slot(&bought).is_some()))
                || self.seats[seat].stash.receiving_slot(&bought).is_some()
        };
        Some(PurchasePlan { parts, cost, fits })
    }

    /// Buys one mergeable charge, with capacity and price checked before mutation.
    fn buy_stackable(
        &mut self,
        seat: usize,
        unit: Option<Entity>,
        item: ItemId,
        events: &mut Vec<Event>,
    ) -> bool {
        let def = item_def(item).expect("known catalog item");
        let owner = self.seats[seat].slot;
        if self.seats[seat].gold < def.cost || !self.purchase_fits(owner, item) {
            return false;
        }
        let bought = ItemStack::bought(item, owner, self.tick).expect("known catalog item");
        let in_hand = self.at_home(unit)
            && unit
                .and_then(|unit| self.inventory.get_mut(unit))
                .is_some_and(|bag| bag.receive(bought));
        if !in_hand {
            assert!(self.seats[seat].stash.receive(bought));
        }
        self.finish_purchase(seat, item, def.cost, events);
        true
    }

    /// Pays for a completed purchase, takes it off the shelf, and emits its
    /// team-visible event.
    fn finish_purchase(&mut self, seat: usize, item: ItemId, price: i32, events: &mut Vec<Event>) {
        assert!(price >= 0);
        assert!(self.seats[seat].gold >= price);
        self.seats[seat].gold -= price;
        self.take_from_stock(self.seats[seat].team, item);
        events.push(Event {
            kind: EventKind::ItemBought {
                slot: self.seats[seat].slot,
                item,
            },
            visible_to: EventVisibility::OneTeam(self.seats[seat].team),
            gold_hidden_from: None,
            source_told_to: EventVisibility::Everyone,
        });
    }

    /// What a seat still has to buy for one item to be had.
    ///
    /// An item bought whole is the whole of the answer, held already or not:
    /// asking for a second one buys a second one. One that is built answers
    /// with the parts it lacks, each of those asked the same question in turn,
    /// so a part that is itself built is bought as its own parts. A part
    /// already waiting in the bag or the stash is spent on the build and so is
    /// bought no second time. Of several recipes the cheapest is bought, see
    /// [`bota_core::missing_parts`].
    /// While the hero lies dead, what it carried when it fell counts as held,
    /// as in Dota.
    fn missing_parts(&self, seat: usize, unit: Option<Entity>, item: ItemId) -> Vec<ItemId> {
        let bag = match unit {
            Some(unit) => self.inventory.get(unit),
            None => self.seats[seat].kept.as_ref().map(|kept| &kept.bag),
        };
        let held: Vec<ItemId> = bag
            .into_iter()
            .flat_map(|bag| bag.slots.iter().take(BAG_SLOTS).flatten())
            .chain(self.seats[seat].stash.slots.iter().flatten())
            .filter(|stack| stack.owner == self.seats[seat].slot && !stack.for_sale)
            .map(|stack| stack.id)
            .collect();
        let primary = match unit {
            Some(unit) => self.def.get(unit).and_then(|kind| kind.0.primary),
            None => bota_core::hero_def(self.seats[seat].hero).and_then(|hero| hero.unit.primary),
        };
        bota_core::missing_parts(item, &held, primary)
    }

    /// Whether a seat's hero stands in its home shop: never while it lies
    /// dead.
    fn at_home(&self, unit: Option<Entity>) -> bool {
        unit.is_some_and(|unit| self.at_shop(unit))
    }

    /// Slots a seat has free for something bought right now.
    fn free_slots(&self, seat: usize, unit: Option<Entity>) -> usize {
        let stash = self.seats[seat]
            .stash
            .slots
            .iter()
            .filter(|slot| slot.is_none())
            .count();
        if !self.at_home(unit) {
            return stash;
        }
        let bag = unit
            .and_then(|unit| self.inventory.get(unit))
            .map_or(0, |bag| bag.slots.iter().filter(|s| s.is_none()).count());
        bag + stash
    }

    pub fn item_def_in(&self, entity: Entity, slot: usize) -> Option<&'static ItemDef> {
        let stack = self.inventory.get(entity)?.slot(slot).copied()??;
        item_def(stack.id)
    }

    pub fn charges_in(&self, entity: Entity, slot: usize) -> u8 {
        self.held_in_bag(entity, slot)
            .map_or(0, |stack| stack.charges)
    }

    /// Runs one use of an item to the moment it has gone off: the checks,
    /// the item's own work, and the cost.
    ///
    /// A use that does nothing spends nothing: the charge, the cooldown and
    /// the slot are only touched once whatever the item does has been done.
    pub fn begin_item(&mut self, entity: Entity, slot: usize, target: Target) -> bool {
        let Some(bag) = self.inventory.get(entity) else {
            return false;
        };
        let Some(Some(stack)) = bag.slot(slot).copied() else {
            return false;
        };
        if !(in_inventory(slot) || in_town_portal(slot)) || stack.cooldown > 0 || stack.mute > 0 {
            return false;
        }
        let Some(def) = item_def(stack.id) else {
            return false;
        };
        if def.aim.is_none() {
            return false;
        }
        if def.shared_wait && self.owes_wait(entity, stack.id) {
            return false;
        }
        if (def.charges > 0 || def.cast_charges > 0) && stack.charges == 0 {
            return false;
        }
        let cost = self.item_mana_cost(entity, stack.id);
        if cost > 0
            && self
                .mana
                .get(entity)
                .is_none_or(|pool| pool.mana < Fixed::from_int(cost))
        {
            return false;
        }
        if !self.use_effect(entity, slot, (stack.id, def), target) {
            return false;
        }
        if cost > 0
            && let Some(pool) = self.mana.get_mut(entity)
        {
            pool.mana -= Fixed::from_int(cost);
        }
        if def.health_cost > 0
            && let Some(pool) = self.health.get_mut(entity)
        {
            pool.hp = (pool.hp - Fixed::from_int(def.health_cost)).max(Fixed::ONE);
        }
        let cooldown = self.item_cooldown(entity, stack.id);
        if def.shared_wait {
            self.owe_wait(entity, stack.id, cooldown);
        }
        if let Some(bag) = self.inventory.get_mut(entity)
            && let Some(held) = bag.slot_mut(slot)
            && let Some(stack) = held
        {
            stack.cooldown = if def.shared_wait { 0 } else { cooldown };
            stack.touched = true;
            stack.charges = match def.spends {
                Spends::Nothing => stack.charges,
                Spends::One => stack.charges.saturating_sub(1),
                Spends::All => 0,
            };
            // What gains charges again is kept when the last one goes;
            // what does not is gone with it.
            if stack.charges == 0 && def.cast_charges == 0 && def.spends != Spends::Nothing {
                *held = None;
            }
        }
        true
    }

    /// Whether self-targeted instant mana restoration has a live user and a positive deficit.
    pub fn can_replenish_mana(&self, user: Entity, target: Target) -> bool {
        if !self.alive(user)
            || !matches!(target, Target::None) && target != Target::Unit(wire_id(user))
        {
            return false;
        }
        let (Some(stats), Some(pool)) = (self.stats.get(user), self.mana.get(user)) else {
            return false;
        };
        stats.max_mana > Fixed::ZERO && pool.mana >= Fixed::ZERO && pool.mana < stats.max_mana
    }

    /// Restores up to `amount` mana immediately; an ineffective use changes nothing.
    pub fn replenish_mana(
        &mut self,
        user: Entity,
        target: Target,
        amount: i32,
        with: Inflictor,
    ) -> bool {
        assert!(amount > 0);
        if !self.can_replenish_mana(user, target) {
            return false;
        }
        let maximum = self
            .stats
            .get(user)
            .expect("validated mana capacity")
            .max_mana;
        let pool = self.mana.get_mut(user).expect("validated mana pool");
        let before = pool.mana;
        pool.mana += (maximum - before).min(Fixed::from_int(amount));
        let restored = (pool.mana - before).to_int();
        if restored > 0 {
            self.tell_healed((user, with), user, 0, restored);
        }
        true
    }

    /// Mends whoever used an item, at once.
    pub fn restore_with(&mut self, on: Entity, (hp, mana): (i32, i32), with: Inflictor) -> bool {
        if hp <= 0 && mana <= 0 {
            return false;
        }
        let ceiling = self.stats.get(on).copied();
        let mut restored = 0;
        if let Some(pool) = self.health.get_mut(on)
            && let Some(most) = ceiling.map(|stats| stats.max_hp)
        {
            let before = pool.hp;
            pool.hp = (pool.hp + Fixed::from_int(hp)).min(most);
            restored = (pool.hp - before).to_int();
        }
        let mut refilled = 0;
        if let Some(pool) = self.mana.get_mut(on)
            && let Some(most) = ceiling.map(|stats| stats.max_mana)
        {
            let before = pool.mana;
            pool.mana = (pool.mana + Fixed::from_int(mana)).min(most);
            refilled = (pool.mana - before).to_int();
        }
        if restored > 0 || refilled > 0 {
            self.tell_healed((on, with), on, restored, refilled);
        }
        true
    }

    /// Walks whoever used an item faster, and through whatever is in the way.
    pub fn walk_through(&mut self, user: Entity, pct: i32, ticks: u32, with: Inflictor) -> bool {
        for kind in [ModifierKind::Hastened { pct }, ModifierKind::Phased] {
            self.put_modifier(
                user,
                Modifier::new(kind, Some(user), Some(with), Some(ticks)),
            );
        }
        true
    }

    /// Sets what sits in a slot to the attribute after the one it is on.
    pub fn switch_mode(&mut self, user: Entity, slot: usize) -> bool {
        let Some(bag) = self.inventory.get_mut(user) else {
            return false;
        };
        let Some(Some(stack)) = bag.slots.get_mut(slot) else {
            return false;
        };
        let Some(mode) = stack.mode else {
            return false;
        };
        // Dota's wheel: strength, intelligence, agility.
        stack.mode = Some(match mode {
            Attribute::Strength => Attribute::Intelligence,
            Attribute::Intelligence => Attribute::Agility,
            Attribute::Agility => Attribute::Strength,
        });
        true
    }

    /// Whether whoever holds this entity still owes a wait on a kind of item.
    pub fn owes_wait(&self, entity: Entity, item: ItemId) -> bool {
        self.seat_of(entity).is_some_and(|seat| {
            self.seats[seat]
                .item_clocks
                .iter()
                .any(|(held, _)| *held == item)
        })
    }

    /// Starts the wait a kind of item owes its user.
    fn owe_wait(&mut self, entity: Entity, item: ItemId, ticks: u32) {
        let Some(seat) = self.seat_of(entity) else {
            return;
        };
        let seat = &mut self.seats[seat];
        seat.item_clocks.retain(|(held, _)| *held != item);
        if ticks > 0 {
            seat.item_clocks.push((item, ticks));
        }
    }

    /// Puts a mending effect on whoever an item was used on.
    ///
    /// It reaches one of its user's own side, standing within `range`. Aimed
    /// at nothing at all, it lands on the one who used it.
    pub fn mend_with(
        &mut self,
        user: Entity,
        target: Target,
        drink: Mend,
        with: Inflictor,
    ) -> bool {
        let Mend {
            pool,
            mut total,
            mut ticks,
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
        let (Some(from), Some(at)) = (self.pos(user), self.pos(on)) else {
            return false;
        };
        if !from.within(at, rules::units(range)) {
            return false;
        }
        // What eats a tree is paid for by one: with none in reach nothing is
        // eaten and nothing is spent. One that was put up feeds twice as long
        // as one the map grew.
        if eats_a_tree {
            let Some(tree) = self.reach_a_tree(user, target, range) else {
                return false;
            };
            if matches!(tree, Tree::Planted(_)) {
                ticks *= 2;
                total *= 2;
            }
            self.fell_tree(tree, user);
        }
        let per_tick = total * 100 / ticks.max(1) as i32;
        self.put_modifier(
            on,
            Modifier::new(
                match pool {
                    Pool::Health => ModifierKind::Mending { per_tick, breaks },
                    Pool::Mana => ModifierKind::Clarity { per_tick, breaks },
                },
                Some(user),
                Some(with),
                Some(ticks),
            ),
        );
        self.tell_mend((user, with), on, pool, total);
        true
    }

    /// Tells of a mend as it begins, worth what `on` is missing of `pool` and
    /// no more than `total`. A mend broken early has still been told in full.
    fn tell_mend(&mut self, by: (Entity, Inflictor), on: Entity, pool: Pool, total: i32) {
        let stats = self.stats.get(on);
        let missing = match pool {
            Pool::Health => stats
                .zip(self.health.get(on))
                .map_or(0, |(stats, health)| (stats.max_hp - health.hp).to_int()),
            Pool::Mana => stats
                .zip(self.mana.get(on))
                .map_or(0, |(stats, pool)| (stats.max_mana - pool.mana).to_int()),
        };
        let mended = total.min(missing.max(0));
        let (amount, mana) = match pool {
            Pool::Health => (mended, 0),
            Pool::Mana => (0, mended),
        };
        if amount > 0 || mana > 0 {
            self.tell_healed(by, on, amount, mana);
        }
    }

    /// Tells who may know that `source` mended `on` with `inflictor` by
    /// `amount` health and `mana` mana.
    fn tell_healed(
        &mut self,
        (source, inflictor): (Entity, Inflictor),
        on: Entity,
        amount: i32,
        mana: i32,
    ) {
        self.events.push(Event {
            kind: EventKind::Healed {
                source: Some(wire_id(source)),
                target: wire_id(on),
                amount,
                mana,
                inflictor: Some(inflictor),
            },
            visible_to: self.who_may_know(on, None),
            gold_hidden_from: None,
            source_told_to: self.who_may_name(Some(source)),
        });
    }

    /// Burns one charge of what sits in a slot, clearing the slot when the
    /// last one goes.
    pub fn spend_charge(&mut self, entity: Entity, slot: usize) {
        if let Some(bag) = self.inventory.get_mut(entity)
            && let Some(held) = bag.slot_mut(slot)
            && let Some(stack) = held
        {
            stack.touched = true;
            stack.spend_one();
            if stack.charges == 0 {
                *held = None;
            }
        }
    }

    /// Moves a stack to another slot, merging compatible charges up to the cap
    /// or swapping whatever is in the way. A partial merge leaves its remainder in place.
    ///
    /// Slots run the unit's own bag first, then the seat's stash. The stash
    /// takes part only while that unit stands in its own shop, a courier as
    /// much as a hero. A stack coming out of the backpack into the inventory
    /// is muted for [`rules::BACKPACK_MUTE_TICKS`].
    pub fn move_item(&mut self, slot: SlotId, unit: Entity, from: usize, to: usize) -> bool {
        let Some(seat) = self.seats.iter().position(|s| s.slot == slot) else {
            return false;
        };
        if self
            .swap_refusal(unit, &self.seats[seat], from, to)
            .is_some()
        {
            return false;
        }
        if !in_stash(from) && !in_stash(to) {
            return self
                .inventory
                .get_mut(unit)
                .is_some_and(|bag| bag.swap_slots(from, to));
        }
        if in_town_portal(to) {
            return self.stow_scroll(unit, seat, from);
        }
        if self.slot_of(unit, seat, from).is_none() || self.slot_of(unit, seat, to).is_none() {
            return false;
        }
        let Some(mut moved) = self.take_slot(unit, seat, from) else {
            return false;
        };
        let displaced = self.take_slot(unit, seat, to);
        if let Some(mut target) = displaced
            && target.merge_from(&mut moved, !in_stash(to))
        {
            target.touched = true;
            moved.touched = true;
            if in_backpack(from) && in_inventory(to) {
                target.mute = target.mute.max(rules::BACKPACK_MUTE_TICKS);
            }
            self.put_slot(unit, seat, to, Some(target));
            self.put_slot(unit, seat, from, (moved.charges > 0).then_some(moved));
            return true;
        }
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

    /// Why two of a unit's slots may not swap what they hold, or nothing when
    /// they may (see [`swap_refusal_in`]): the stash takes part only while the
    /// unit stands at its own shop.
    pub(crate) fn swap_refusal(
        &self,
        unit: Entity,
        seat: &crate::game::Seat,
        from: usize,
        to: usize,
    ) -> Option<bota_proto::RejectReason> {
        swap_refusal_in(
            self.inventory.get(unit),
            seat,
            (from, to),
            self.at_shop(unit),
        )
    }

    /// The stack in one of a seat's slots: the unit's bag, then the stash.
    fn stack_at(&self, unit: Entity, seat: &crate::game::Seat, at: usize) -> Option<ItemStack> {
        if in_stash(at) {
            seat.stash.slots.get(at - BAG_SLOTS).copied().flatten()
        } else {
            self.held_in_bag(unit, at)
        }
    }

    /// Merges a scroll stack from one of a seat's slots into its hero's town
    /// portal slot, whole or not at all.
    fn stow_scroll(&mut self, unit: Entity, seat: usize, from: usize) -> bool {
        let Some(mut moved) = self.take_slot(unit, seat, from) else {
            return false;
        };
        assert!(moved.is_scroll());
        moved.touched = true;
        let taken = self
            .inventory
            .get_mut(unit)
            .is_some_and(|bag| bag.receive(moved));
        if !taken {
            self.put_slot(unit, seat, from, Some(moved));
        }
        taken
    }

    /// Sells what sits in one slot, or marks it to be sold.
    ///
    /// The stash sells from wherever its owner is: it is already at the shop.
    /// A unit's own bag sells only while that unit stands there; anywhere
    /// else the order marks the stack for sale, and a second order on the
    /// slot unmarks it. Only the seat that bought a stack may sell or mark
    /// it.
    ///
    /// An untouched stack sold within [`rules::SELL_REFUND_TICKS`] of its
    /// purchase pays back what it cost; anything else pays back
    /// [`rules::SELL_PCT`] of it.
    pub fn sell_item(&mut self, slot: SlotId, unit: Entity, at: usize) -> bool {
        let Some(seat) = self.seats.iter().position(|s| s.slot == slot) else {
            return false;
        };
        let Some(held) = self.stack_at(unit, &self.seats[seat], at) else {
            return false;
        };
        if held.owner != slot || held.free_charges > 0 {
            return false;
        }
        if in_town_portal(at) && !self.at_shop(unit) {
            return false;
        }
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

    /// An order of a seat whose hero lies dead: a purchase, into the stash,
    /// or the sale of a stack bought since the hero fell; nothing else is
    /// carried out without a body (see [`World::validate_order`]).
    pub(crate) fn take_order_while_down(
        &mut self,
        slot: SlotId,
        order: bota_proto::Order,
        events: &mut Vec<Event>,
    ) {
        match order {
            bota_proto::Order::Buy { item } => {
                self.buy(slot, item, events);
            }
            bota_proto::Order::Swap { from, to } => self.swap_while_down(slot, from, to),
            bota_proto::Order::Sell { slot: at } => self.sell_while_down(slot, usize::from(at.0)),
            bota_proto::Order::Cheat { cheat } => self.cheat(slot, None, cheat, events),
            _ => {}
        }
    }

    /// Swaps two slots of what a dead hero carried, as it could standing away
    /// from the shop: its bag's, never the stash's.
    fn swap_while_down(
        &mut self,
        slot: SlotId,
        from: bota_proto::ItemSlot,
        to: bota_proto::ItemSlot,
    ) {
        let Some(seat) = self.seats.iter().position(|s| s.slot == slot) else {
            return;
        };
        let (from, to) = (usize::from(from.0), usize::from(to.0));
        let seat = &mut self.seats[seat];
        let Some(kept) = seat.kept.as_ref() else {
            return;
        };
        if swap_refusal_in(Some(&kept.bag), seat, (from, to), false).is_some() {
            return;
        }
        if let Some(kept) = seat.kept.as_mut() {
            kept.bag.swap_slots(from, to);
        }
    }

    fn sell_while_down(&mut self, slot: SlotId, at: usize) {
        let Some(seat) = self.seats.iter().position(|s| s.slot == slot) else {
            return;
        };
        let Ok(stash_slot) = self.sale_while_down(&self.seats[seat], at) else {
            return;
        };
        let Some(stack) = self.seats[seat].stash.slots[stash_slot].take() else {
            return;
        };
        let back = self.sale_price(&stack);
        self.seats[seat].gold += back;
    }

    fn sale_price(&self, stack: &ItemStack) -> i32 {
        let Some(def) = item_def(stack.id) else {
            return 0;
        };
        let fresh = !stack.touched
            && self.tick.saturating_sub(stack.bought_tick) <= rules::SELL_REFUND_TICKS;
        let cost = match stack.ward_counts() {
            // A dispenser is worth the wards it holds, each at its own price.
            Some((observers, sentries)) => {
                let price = |ward: ItemId, count: u8| {
                    item_def(ward).map_or(0, |def| def.cost) * i32::from(count)
                };
                price(ItemId::OBSERVER_WARD, observers) + price(ItemId::SENTRY_WARD, sentries)
            }
            None if def.stack_limit > 0 => {
                assert!(stack.charges <= def.stack_limit);
                def.cost * i32::from(stack.charges)
            }
            None => def.cost,
        };
        if fresh {
            cost
        } else {
            cost * rules::SELL_PCT / 100
        }
    }

    /// Sells every stack marked for sale that has reached the shop.
    ///
    /// A marked stack sells in a stash, or in any bag standing within
    /// [`rules::SHOP_RANGE`] of its owner's fountain, however it got there;
    /// the gold goes to its owner.
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
            let Some(at_pos) = self.pos(holder) else {
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
                let shop = fountain_pos(self.map, self.seats[owner].team);
                if !at_pos.within(shop, rules::units(rules::SHOP_RANGE)) {
                    continue;
                }
                let back = self.sale_price(&stack);
                self.take_from_bag(holder, at);
                self.seats[owner].gold += back;
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    fn take_slot(&mut self, unit: Entity, seat: usize, slot: usize) -> Option<ItemStack> {
        self.slot_of(unit, seat, slot)?.take()
    }

    /// Lays a stack in one slot, over whatever was there.
    fn put_slot(&mut self, unit: Entity, seat: usize, slot: usize, stack: Option<ItemStack>) {
        if let Some(held) = self.slot_of(unit, seat, slot) {
            *held = stack;
        }
    }

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
            self.inventory.get_mut(unit)?.slot_mut(slot)
        }
    }

    /// Carries whoever used an item to a point.
    ///
    /// Aimed further off than `range`, it carries `overshoot` along the same
    /// line, as Dota's dagger lands short of its range when clicked past it.
    /// A landing spot on closed ground steps back along that line until it
    /// finds open ground, and the blink fails when it finds none.
    pub fn blink_to(&mut self, user: Entity, target: Target, range: i32, overshoot: i32) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let Some(from) = self.pos(user) else {
            return false;
        };
        let aim = clamp_to_map(if from.within(pos, rules::units(range)) {
            pos
        } else {
            move_towards(from, pos, rules::units(overshoot))
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
            if self.clearance.stands_clear(at) {
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
    /// The tree nearest the spot aimed at, within [`rules::TREE_RADIUS`] of
    /// it, and within `range` of the one using the item. Aimed at a unit, the
    /// spot is where the unit stands; aimed at nothing, where the user does.
    fn reach_a_tree(&self, user: Entity, target: Target, range: i32) -> Option<Tree> {
        let from = self.pos(user)?;
        let at = match target {
            Target::Pos(pos) => pos,
            Target::Unit(target) => self.pos(self.of_wire(target)?)?,
            Target::None => from,
        };
        let tree = self.trees.nearest(at, rules::units(rules::TREE_RADIUS))?;
        let spot = self.trees.spot(tree)?;
        from.within(spot, rules::units(range)).then_some(tree)
    }

    /// Takes down the tree an item was aimed at.
    pub fn fell_a_tree(&mut self, user: Entity, target: Target, range: i32) -> bool {
        let Some(tree) = self.reach_a_tree(user, target, range) else {
            return false;
        };
        self.fell_tree(tree, user);
        true
    }

    /// Puts a tree up where an item was aimed, on ground that has none.
    pub fn plant_a_tree(&mut self, user: Entity, target: Target, ticks: u32, range: i32) -> bool {
        let Target::Pos(pos) = target else {
            return false;
        };
        let Some(from) = self.pos(user) else {
            return false;
        };
        if !from.within(pos, rules::units(range)) || !self.clearance.stands_clear(pos) {
            return false;
        }
        // Not on top of a tree already standing there.
        if self
            .trees
            .nearest(pos, rules::units(rules::TREE_RADIUS))
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
    /// others (the razes) levels every slot of the group at once.
    pub fn learn(&mut self, entity: Entity, slot: usize) -> bool {
        let hero_level = self.level.get(entity).map_or(1, |level| level.0);
        let Some(book) = self.abilities.get_mut(entity) else {
            return false;
        };
        if !book.can_raise(slot, hero_level) {
            return false;
        }
        let group = learn_group(book.slots[slot].id);
        for held in &mut book.slots {
            if learn_group(held.id) == group {
                held.level += 1;
            }
        }
        true
    }

    /// Takes a talent of an entity's tree, when [`World::can_learn_talent`]
    /// allows it.
    pub fn learn_talent(&mut self, entity: Entity, slot: bota_proto::TalentSlot) -> bool {
        if !self.can_learn_talent(entity, slot) {
            return false;
        }
        let Some(book) = self.abilities.get_mut(entity) else {
            return false;
        };
        book.talents = book.talents.with(slot);
        true
    }

    /// Whether a talent could be taken right now: the entity's tree has it,
    /// neither it nor (below [`rules::TALENT_SECOND_LEVELS`]) the other of
    /// its tier is taken, and the hero level has reached the one it opens
    /// at. Since Dota 7.40 a talent spends no skill point: "talents are
    /// learned by using their own talent points available at levels 10, 15,
    /// 20, 25, 27, 28, 29, and 30", so the opening levels are the whole
    /// count and no point counter is kept.
    pub fn can_learn_talent(&self, entity: Entity, slot: bota_proto::TalentSlot) -> bool {
        let Some(book) = self.abilities.get(entity) else {
            return false;
        };
        let hero_level = self.level.get(entity).map_or(0, |level| level.0);
        book.tree.is_some()
            && slot.0 < bota_proto::TalentSlot::COUNT
            && !book.talents.has(slot)
            && hero_level >= bota_core::talent_floor(slot, book.talents)
    }

    /// Spends a skill point on an entity's Attribute Bonus, when
    /// [`World::can_learn_attribute_bonus`] allows it.
    pub fn learn_attribute_bonus(&mut self, entity: Entity) -> bool {
        let hero_level = self.level.get(entity).map_or(0, |level| level.0);
        self.abilities
            .get_mut(entity)
            .is_some_and(|book| book.learn_attribute_bonus(hero_level))
    }

    /// Whether a skill point could go into an entity's Attribute Bonus
    /// right now, see [`AbilityBook::can_learn_attribute_bonus`].
    pub fn can_learn_attribute_bonus(&self, entity: Entity) -> bool {
        let hero_level = self.level.get(entity).map_or(0, |level| level.0);
        self.abilities
            .get(entity)
            .is_some_and(|book| book.can_learn_attribute_bonus(hero_level))
    }

    /// Skill points an entity has spent, see [`AbilityBook::points_spent`].
    pub fn points_spent(&self, entity: Entity) -> u8 {
        self.abilities
            .get(entity)
            .map_or(0, AbilityBook::points_spent)
    }
}

/// Why two slots may not swap what they hold, or nothing when they may: of
/// `bag`, a standing hero's or a dead one's, and of the seat's stash, which
/// takes part only `at_shop`. The town portal slot takes a scroll stack from
/// another slot, merging it, and gives nothing up: Dota's scrolls never leave
/// it but to be read.
pub(crate) fn swap_refusal_in(
    bag: Option<&Inventory>,
    seat: &crate::game::Seat,
    (from, to): (usize, usize),
    at_shop: bool,
) -> Option<bota_proto::RejectReason> {
    use bota_proto::RejectReason;
    let total = BAG_SLOTS + seat.stash.slots.len();
    let named = |at: usize| at < total || in_town_portal(at);
    if from == to || !named(from) || !named(to) {
        return Some(RejectReason::EmptySlot);
    }
    let held = if in_stash(from) {
        seat.stash.slots.get(from - BAG_SLOTS).copied().flatten()
    } else {
        bag.and_then(|bag| bag.slot(from).copied().flatten())
    };
    let Some(held) = held else {
        return Some(RejectReason::EmptySlot);
    };
    if (in_stash(from) || in_stash(to)) && !at_shop {
        return Some(RejectReason::NotAtShop);
    }
    let has_portal = bag.is_some_and(|bag| bag.town_portal.is_some());
    if in_town_portal(from) || (in_town_portal(to) && (!has_portal || !held.is_scroll())) {
        return Some(RejectReason::WrongTargetKind);
    }
    None
}
