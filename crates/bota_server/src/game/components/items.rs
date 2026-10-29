//! What an entity carries.

use bota_core::{item_def, rules};
use bota_proto::{Attribute, ItemId, SlotId};

/// One item in a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    pub id: ItemId,
    /// Uses left, for a consumable. Zero for one that only carries bonuses.
    pub charges: u8,
    /// Ticks until it may be used again.
    pub cooldown: u32,
    /// Ticks it stays inert for having come out of the backpack: it carries
    /// nothing and cannot be used until this runs out.
    pub mute: u32,
    /// Which attribute it is set to. Absent for one that is set to none.
    pub mode: Option<Attribute>,
    /// The tick it was bought, for the window in which it refunds in full.
    pub bought_tick: u32,
    /// Whether it has been used or moved since it was bought.
    pub touched: bool,
    /// The seat that bought it. Only this seat may sell or mark it.
    pub owner: SlotId,
    /// Whether it is to be sold when it next reaches the shop.
    pub for_sale: bool,
    /// Charges of it that came for nothing: the scroll every hero starts
    /// with and the ones deaths hand out. They are spent first, and while
    /// any is held no shop buys the stack back.
    pub free_charges: u8,
    /// The Sentry Wards a Ward Dispenser holds beside `charges`, its
    /// Observer Wards. Zero for any other item.
    pub sentries: u8,
    /// Whether a Ward Dispenser plants a Sentry Ward next rather than an
    /// Observer Ward. False for any other item.
    pub sentry_active: bool,
}

impl ItemStack {
    /// An untouched purchase with the catalog's initial charges and mode.
    /// Nothing for an item no shop sells.
    pub fn bought(id: ItemId, owner: SlotId, tick: u32) -> Option<Self> {
        let def = item_def(id).filter(|def| !def.unbuyable)?;
        if def.stack_limit > 0 {
            assert!(def.charges >= 1 && def.charges <= def.stack_limit);
        }
        Some(Self {
            id,
            charges: def.charges,
            cooldown: 0,
            mute: 0,
            mode: def.mode,
            bought_tick: tick,
            touched: false,
            owner,
            for_sale: false,
            free_charges: 0,
            sentries: 0,
            sentry_active: false,
        })
    }

    /// One Town Portal Scroll that came for nothing.
    pub fn free_scroll(owner: SlotId, tick: u32) -> Self {
        let mut scroll = Self::bought(ItemId::TOWN_PORTAL_SCROLL, owner, tick)
            .expect("the catalog sells scrolls");
        assert_eq!(scroll.charges, 1);
        scroll.free_charges = 1;
        scroll
    }

    /// Whether it is a Town Portal Scroll, the one thing a town portal slot
    /// holds.
    pub fn is_scroll(&self) -> bool {
        self.id == ItemId::TOWN_PORTAL_SCROLL
    }

    /// Spends one charge, one that came for nothing first.
    pub fn spend_one(&mut self) {
        assert!(self.free_charges <= self.charges);
        self.charges = self.charges.saturating_sub(1);
        self.free_charges = self.free_charges.saturating_sub(1);
        assert!(self.free_charges <= self.charges);
    }

    /// How many Observer Wards and Sentry Wards a stack of the ward family
    /// holds: the two wards and the Ward Dispenser they make. Nothing for
    /// any other item.
    pub fn ward_counts(&self) -> Option<(u8, u8)> {
        match self.id {
            ItemId::OBSERVER_WARD => Some((self.charges, 0)),
            ItemId::SENTRY_WARD => Some((0, self.charges)),
            ItemId::WARD_DISPENSER => Some((self.charges, self.sentries)),
            _ => None,
        }
    }

    /// The charges a use would spend: those of the active kind of ward in a
    /// Ward Dispenser, the stack's own for any other item.
    pub fn active_charges(&self) -> u8 {
        if self.id == ItemId::WARD_DISPENSER && self.sentry_active {
            self.sentries
        } else {
            self.charges
        }
    }

    /// The ward item a Ward Dispenser plants next.
    pub fn active_ward(&self) -> ItemId {
        if self.sentry_active {
            ItemId::SENTRY_WARD
        } else {
            ItemId::OBSERVER_WARD
        }
    }

    /// Switches the kind of ward a Ward Dispenser plants next.
    pub fn switch_ward(&mut self) {
        assert_eq!(self.id, ItemId::WARD_DISPENSER);
        self.sentry_active = !self.sentry_active;
    }

    /// Plants one ward out of a Ward Dispenser. A kind that runs out leaves
    /// the other as the plain ward it is, as Dota's dispenser does.
    pub fn spend_ward(&mut self) {
        assert_eq!(self.id, ItemId::WARD_DISPENSER);
        if self.sentry_active {
            self.sentries -= 1;
        } else {
            self.charges -= 1;
        }
        let (observers, sentries) = (self.charges, self.sentries);
        let (id, charges) = match (observers, sentries) {
            (0, sentries) => (ItemId::SENTRY_WARD, sentries),
            (observers, 0) => (ItemId::OBSERVER_WARD, observers),
            _ => return,
        };
        self.id = id;
        self.charges = charges;
        self.sentries = 0;
        self.sentry_active = false;
    }

    /// Charges another stack may add; ownership, mode, and sale marks must
    /// match.
    ///
    /// Of one kind that stacks, up to its cap. With `combine`, the two
    /// wards and the Ward Dispenser also merge into one dispenser, whole or
    /// not at all, as far as neither count passes a byte: `u8::MAX` when the
    /// whole of the source fits. A stash builds nothing, so it passes false.
    pub fn merge_room(&self, source: &Self, combine: bool) -> u8 {
        if self.owner != source.owner
            || self.mode != source.mode
            || self.for_sale != source.for_sale
            || self.charges == 0
            || source.charges == 0
        {
            return 0;
        }
        let limit = item_def(self.id).map_or(0, |def| def.stack_limit);
        if self.id == source.id && limit > 0 {
            assert!(self.charges <= limit);
            assert!(source.charges <= limit);
            return limit - self.charges;
        }
        let (Some((o1, s1)), Some((o2, s2))) = (self.ward_counts(), source.ward_counts()) else {
            return 0;
        };
        let fits = o1.checked_add(o2).is_some() && s1.checked_add(s2).is_some();
        if combine && fits { u8::MAX } else { 0 }
    }

    /// Merges up to the cap, retaining the oldest purchase and strongest restrictions.
    /// A source reduced to zero must be removed from its slot by the caller.
    pub fn merge_from(&mut self, source: &mut Self, combine: bool) -> bool {
        let room = self.merge_room(source, combine);
        if room == 0 {
            return false;
        }
        if self.id == source.id && item_def(self.id).is_some_and(|def| def.stack_limit > 0) {
            let moved = room.min(source.charges);
            self.charges += moved;
            source.charges -= moved;
            // Free charges travel with the charges that carry them, and stay
            // within the charges either side holds.
            let free = moved.min(source.free_charges);
            self.free_charges += free;
            source.free_charges -= free;
            self.settle(source);
            assert!(self.free_charges <= self.charges);
            assert!(source.free_charges <= source.charges);
            return true;
        }
        let (o1, s1) = self.ward_counts().expect("a ward");
        let (o2, s2) = source.ward_counts().expect("a ward");
        if self.id != ItemId::WARD_DISPENSER {
            self.sentry_active = source.sentry_active;
        }
        self.id = ItemId::WARD_DISPENSER;
        self.charges = o1 + o2;
        self.sentries = s1 + s2;
        source.charges = 0;
        source.sentries = 0;
        self.settle(source);
        true
    }

    /// What a merge keeps of two stacks beyond their charges.
    fn settle(&mut self, source: &Self) {
        self.bought_tick = self.bought_tick.min(source.bought_tick);
        self.cooldown = self.cooldown.max(source.cooldown);
        self.mute = self.mute.max(source.mute);
        self.touched |= source.touched;
    }
}

/// The slots an entity carries items in.
///
/// A slot holding nothing is `None`, so slots keep their numbers as items come
/// and go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    /// Every slot, in the order they are shown.
    pub slots: Vec<Option<ItemStack>>,
    /// A hero's town portal slot, numbered [`rules::TOWN_PORTAL_SLOT`]:
    /// what it holds, a scroll stack or nothing. Absent for a bag that has
    /// none, a courier's or the stash. Kept apart from `slots` so that no
    /// search for a free slot can put anything but a scroll in it.
    pub town_portal: Option<Option<ItemStack>>,
    /// Whether it is a seat's stash: it holds, and builds nothing, so the
    /// wards in it never come together into a Ward Dispenser.
    pub stash: bool,
}

impl Inventory {
    /// An inventory of that many empty slots and no town portal slot.
    pub fn empty(slots: usize) -> Inventory {
        Inventory {
            slots: vec![None; slots],
            town_portal: None,
            stash: false,
        }
    }

    /// A seat's stash of that many empty slots.
    pub fn stash(slots: usize) -> Inventory {
        Inventory {
            stash: true,
            ..Inventory::empty(slots)
        }
    }

    /// A hero's bag: the inventory, the backpack and the town portal slot,
    /// that one holding the scroll every hero starts with.
    pub fn of_hero(owner: SlotId, tick: u32) -> Inventory {
        Inventory {
            slots: vec![None; BAG_SLOTS],
            town_portal: Some(Some(ItemStack::free_scroll(owner, tick))),
            stash: false,
        }
    }

    /// Every item held in `slots`, in slot order; the town portal slot apart.
    pub fn held(&self) -> impl Iterator<Item = &ItemStack> {
        self.slots.iter().flatten()
    }

    /// Every stack, the town portal slot's last, to be changed in place.
    pub fn every_stack_mut(&mut self) -> impl Iterator<Item = &mut ItemStack> {
        self.slots
            .iter_mut()
            .chain(self.town_portal.iter_mut())
            .flatten()
    }

    /// One slot by its number, the town portal slot included.
    pub fn slot(&self, at: usize) -> Option<&Option<ItemStack>> {
        if at == rules::TOWN_PORTAL_SLOT {
            return self.town_portal.as_ref();
        }
        self.slots.get(at)
    }

    /// One slot by its number, to be changed in place.
    pub fn slot_mut(&mut self, at: usize) -> Option<&mut Option<ItemStack>> {
        if at == rules::TOWN_PORTAL_SLOT {
            return self.town_portal.as_mut();
        }
        self.slots.get_mut(at)
    }

    /// Where a whole arriving stack would go: a scroll into the town portal
    /// slot when the bag has one, to merge or to fill it; anything else into
    /// the first compatible stack it fits whole, then the first empty slot.
    pub fn receiving_slot(&self, incoming: &ItemStack) -> Option<usize> {
        if incoming.is_scroll()
            && let Some(held) = &self.town_portal
        {
            let fits = held.is_none_or(|held| held.merge_room(incoming, false) >= incoming.charges);
            return fits.then_some(rules::TOWN_PORTAL_SLOT);
        }
        let merge = self.slots.iter().position(|held| {
            held.is_some_and(|held| {
                incoming.charges > 0 && held.merge_room(incoming, !self.stash) >= incoming.charges
            })
        });
        merge.or_else(|| self.slots.iter().position(Option::is_none))
    }

    /// Receives one whole stack or changes nothing; merges only when the entire stack fits.
    pub fn receive(&mut self, mut incoming: ItemStack) -> bool {
        let Some(at) = self.receiving_slot(&incoming) else {
            return false;
        };
        let combine = !self.stash;
        let slot = self.slot_mut(at).expect("a receiving slot exists");
        if let Some(held) = slot.as_mut() {
            assert!(held.merge_from(&mut incoming, combine));
            assert_eq!(incoming.charges, 0);
        } else {
            *slot = Some(incoming);
        }
        true
    }

    /// Swaps what two of its own slots hold, as an order to swap them does: a
    /// stack moved onto a like one merges into it as far as it fits, a scroll
    /// moved to the town portal slot merges there whole or not at all, and a
    /// stack brought out of the backpack waits
    /// [`rules::BACKPACK_MUTE_TICKS`] before it may be used. False when
    /// nothing moved.
    pub fn swap_slots(&mut self, from: usize, to: usize) -> bool {
        let Some(mut moved) = self.slot(from).copied().flatten() else {
            return false;
        };
        let Some(displaced) = self.slot(to).copied() else {
            return false;
        };
        if to == rules::TOWN_PORTAL_SLOT {
            moved.touched = true;
            if self.receiving_slot(&moved) != Some(rules::TOWN_PORTAL_SLOT) {
                return false;
            }
            *self.slot_mut(from).expect("a slot") = None;
            return self.receive(moved);
        }
        let mute = |stack: &mut ItemStack, origin: usize, landed: usize| {
            stack.touched = true;
            if in_backpack(origin) && in_inventory(landed) {
                stack.mute = stack.mute.max(rules::BACKPACK_MUTE_TICKS);
            }
        };
        if let Some(mut target) = displaced
            && target.merge_from(&mut moved, !self.stash)
        {
            moved.touched = true;
            mute(&mut target, from, to);
            *self.slot_mut(to).expect("a slot") = Some(target);
            *self.slot_mut(from).expect("a slot") = (moved.charges > 0).then_some(moved);
            return true;
        }
        let mut displaced = displaced;
        mute(&mut moved, from, to);
        if let Some(stack) = displaced.as_mut() {
            mute(stack, to, from);
        }
        *self.slot_mut(to).expect("a slot") = Some(moved);
        *self.slot_mut(from).expect("a slot") = displaced;
        true
    }

    /// Brings together the wards a bag holds of one seat: an Observer Ward
    /// and a Sentry Ward make a Ward Dispenser, a dispenser takes in the
    /// wards and the dispensers beside it, each into the lowest slot of
    /// them. A stash brings together nothing. True when a slot changed.
    pub fn combine_wards(&mut self) -> bool {
        if self.stash {
            return false;
        }
        let mut changed = false;
        for to in 0..self.slots.len() {
            for from in to + 1..self.slots.len() {
                let (Some(mut target), Some(mut moved)) = (self.slots[to], self.slots[from]) else {
                    continue;
                };
                if target.ward_counts().is_none() || !target.merge_from(&mut moved, true) {
                    continue;
                }
                self.slots[to] = Some(target);
                self.slots[from] = (moved.charges > 0).then_some(moved);
                changed = true;
            }
        }
        changed
    }
}

/// Slots a hero carries on itself: the inventory proper and the backpack.
pub const BAG_SLOTS: usize = bota_core::rules::INVENTORY_SLOTS + bota_core::rules::BACKPACK_SLOTS;

/// Whether a slot number is one of the inventory proper, where items work.
pub fn in_inventory(slot: usize) -> bool {
    slot < bota_core::rules::INVENTORY_SLOTS
}

/// Whether a slot number is one of the backpack, where they are carried inert.
pub fn in_backpack(slot: usize) -> bool {
    (bota_core::rules::INVENTORY_SLOTS..BAG_SLOTS).contains(&slot)
}

/// Whether a slot number is one of the stash at the shop.
pub fn in_stash(slot: usize) -> bool {
    (BAG_SLOTS..bota_core::rules::TOWN_PORTAL_SLOT).contains(&slot)
}

/// Whether a slot number is the town portal slot.
pub fn in_town_portal(slot: usize) -> bool {
    slot == bota_core::rules::TOWN_PORTAL_SLOT
}
