//! What an entity carries.

use bota_core::{item_def, rules};
use bota_proto::{Attribute, ItemId, SlotId};

use crate::engine::state_hash;

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
    /// Whether the owner locked it against combining: assembly and purchases
    /// pass it by as a part.
    pub locked: bool,
    /// The tick up to which a build of it may be taken apart, when its rule
    /// is [`bota_core::DisassembleRule::Window`]: set when it is combined,
    /// zero once it is used or leaves its owner's hands, and zero for
    /// anything that did not come of a combine.
    pub disassemble_until: u32,
    /// Ticks left before it is gone, for a stack of an item with a
    /// [`lifetime`](bota_core::ItemDef::lifetime). Zero for one that lasts.
    pub life: u32,
    /// The rune a Bottle holds. None for any other item.
    pub rune: Option<crate::game::HeldRune>,
}

impl ItemStack {
    /// An untouched purchase with the catalog's initial charges and mode.
    /// Nothing for an item no shop sells.
    pub fn bought(id: ItemId, owner: SlotId, tick: u32) -> Option<Self> {
        let def = item_def(id).filter(|def| !def.unbuyable)?;
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
            locked: false,
            disassemble_until: 0,
            life: def.lifetime,
            rune: None,
        })
    }

    /// What [`ItemStack::disassemble_until`] reads when nothing has ended the
    /// window of a build: ten seconds from its being combined, which the tick
    /// it was bought at (set at the combine) names. Zero for any other stack.
    pub fn default_window(&self) -> u32 {
        match item_def(self.id) {
            Some(def)
                if !def.components.is_empty()
                    && def.disassemble == bota_core::DisassembleRule::Window =>
            {
                self.bought_tick + bota_core::rules::DISASSEMBLY_WINDOW_TICKS
            }
            _ => 0,
        }
    }

    /// One Town Portal Scroll that came for nothing.
    pub fn free_scroll(owner: SlotId, tick: u32) -> Self {
        // The catalog's build-time checks prove the scroll is sold, for one charge.
        let mut scroll = Self::bought(ItemId::TOWN_PORTAL_SCROLL, owner, tick)
            .expect("the catalog sells scrolls");
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
        bota_core::ward_counts(self.id, self.charges, self.sentries)
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

    /// Parts one charge off the stack as a stack of its own, the stack keeping the rest (which may
    /// be none: the caller clears an empty slot): one ward of the kind a Ward Dispenser is
    /// switched to, one charge of any other stack. The part is `into` when that names an item,
    /// which is made fresh and given for nothing, so no shop buys it back. Nothing when the stack
    /// holds no charge.
    pub fn part_one(&mut self, into: Option<ItemId>) -> Option<Self> {
        if self.charges == 0 {
            return None;
        }
        let mut one = Self {
            charges: 1,
            free_charges: 0,
            sentries: 0,
            sentry_active: false,
            touched: true,
            locked: false,
            disassemble_until: 0,
            ..*self
        };
        if self.id == ItemId::WARD_DISPENSER {
            one.id = self.active_ward();
            self.spend_ward();
        } else {
            self.spend_one();
        }
        self.touched = true;
        if let Some(made) = into.and_then(|item| item_def(item).map(|def| (item, def))) {
            one.id = made.0;
            one.cooldown = 0;
            one.mode = made.1.mode;
            one.free_charges = 1;
            one.life = made.1.lifetime;
        }
        Some(one)
    }

    /// Charges another stack may add: [`bota_core::merge_room`], the rule a client asks too.
    pub fn merge_room(&self, source: &Self, combine: bool) -> u8 {
        bota_core::merge_room(self, source, combine)
    }

    /// Merges up to the cap, retaining the oldest purchase and strongest restrictions.
    /// A source reduced to zero must be removed from its slot by the caller.
    pub fn merge_from(&mut self, source: &mut Self, combine: bool) -> bool {
        let room = self.merge_room(source, combine);
        if room == 0 {
            return false;
        }
        let counts = |stack: &Self| bota_core::Counts {
            id: stack.id,
            charges: stack.charges,
            sentries: stack.sentries,
        };
        let (merged, left) = bota_core::merge_counts(counts(self), counts(source), room);
        if self.id == source.id && item_def(self.id).is_some_and(|def| def.stack_limit > 0) {
            let moved = source.charges - left.charges;
            self.charges = merged.charges;
            source.charges = left.charges;
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
        if self.id != ItemId::WARD_DISPENSER {
            self.sentry_active = source.sentry_active;
        }
        self.id = merged.id;
        self.charges = merged.charges;
        self.sentries = merged.sentries;
        source.charges = left.charges;
        source.sentries = left.sentries;
        self.settle(source);
        true
    }

    /// What a merge keeps of two stacks beyond their charges.
    fn settle(&mut self, source: &Self) {
        self.bought_tick = self.bought_tick.min(source.bought_tick);
        self.cooldown = self.cooldown.max(source.cooldown);
        self.mute = self.mute.max(source.mute);
        self.touched |= source.touched;
        self.locked |= source.locked;
    }
}

impl bota_core::HeldItem for ItemStack {
    fn id(&self) -> ItemId {
        self.id
    }

    fn charges(&self) -> u8 {
        self.charges
    }

    fn sentries(&self) -> u8 {
        self.sentries
    }

    fn owner(&self) -> SlotId {
        self.owner
    }

    fn mode(&self) -> Option<Attribute> {
        self.mode
    }

    fn for_sale(&self) -> bool {
        self.for_sale
    }

    fn locked(&self) -> bool {
        self.locked
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

    /// This container as the placing rules read it.
    pub fn shelf(&self) -> bota_core::Shelf<'_, ItemStack> {
        bota_core::Shelf {
            slots: &self.slots,
            town_portal: self.town_portal.as_ref(),
            combines: !self.stash,
        }
    }

    /// Where a whole arriving stack would go: [`bota_core::Shelf::receiving_slot`].
    pub fn receiving_slot(&self, incoming: &ItemStack) -> Option<usize> {
        self.shelf().receiving_slot(incoming)
    }

    /// Receives one whole stack or changes nothing; merges only when the entire stack fits.
    pub fn receive(&mut self, incoming: ItemStack) -> bool {
        let Some(at) = self.receiving_slot(&incoming) else {
            return false;
        };
        self.put_in(at, incoming);
        true
    }

    /// Puts a stack into slot `at`, which [`Inventory::receiving_slot`] named for it: merged into
    /// what is there, or in an empty slot.
    pub fn put_in(&mut self, at: usize, mut incoming: ItemStack) {
        let combine = !self.stash;
        let slot = self.slot_mut(at).expect("a receiving slot exists");
        if let Some(held) = slot.as_mut() {
            assert!(held.merge_from(&mut incoming, combine));
            assert_eq!(incoming.charges, 0);
        } else {
            *slot = Some(incoming);
        }
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

    /// Runs the life of every stack that has one a tick down, and clears the slot of one that
    /// has run out.
    pub fn lapse(&mut self) {
        let run = |slot: &mut Option<ItemStack>| {
            if let Some(stack) = slot
                && stack.life > 0
            {
                stack.life -= 1;
                if stack.life == 0 {
                    *slot = None;
                }
            }
        };
        self.slots.iter_mut().for_each(run);
        self.town_portal.iter_mut().for_each(run);
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

state_hash!(ItemStack {
    id,
    charges,
    cooldown,
    mute,
    mode,
    bought_tick,
    touched,
    owner,
    for_sale,
    free_charges,
    sentries,
    sentry_active,
    locked,
    disassemble_until,
    life,
    rune
});
state_hash!(Inventory {
    slots,
    town_portal,
    stash
});
