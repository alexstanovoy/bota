//! What an entity carries.

use bota_core::{item_def, rules};
use bota_proto::{Attribute, ItemId, SlotId};

use crate::engine::state_hash;

/// Per-item, per-hero first-entry grace (7.38, extended to five seconds in 7.38b).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackpackHistory {
    seen: [u64; 4],
    holder: Option<SlotId>,
    until: u32,
}

impl BackpackHistory {
    pub fn used(&self, hero: SlotId) -> bool {
        self.seen[usize::from(hero.0) / 64] & (1_u64 << (hero.0 % 64)) != 0
    }

    pub fn enter(&mut self, hero: SlotId, now: u32) {
        if self.holder == Some(hero) {
            return;
        }
        self.until = if self.used(hero) {
            0
        } else {
            now.saturating_add(rules::BACKPACK_GRACE_TICKS)
        };
        self.seen[usize::from(hero.0) / 64] |= 1_u64 << (hero.0 % 64);
        self.holder = Some(hero);
    }

    /// Keep consumed grants when stacks merge or contribute to a build.
    pub fn inherit(&mut self, source: Self) {
        for (seen, other) in self.seen.iter_mut().zip(source.seen) {
            *seen |= other;
        }
    }

    /// A new build cannot extend a used ingredient's current grace.
    pub fn inherit_build(&mut self, source: Self) {
        if let Some(hero) = self.holder.filter(|hero| source.used(*hero)) {
            let until = if source.holder == Some(hero) {
                source.until
            } else {
                0
            };
            self.until = self.until.min(until);
        }
        self.inherit(source);
    }

    pub fn leave(&mut self) {
        self.holder = None;
        self.until = 0;
    }

    pub fn left(&self, now: u32) -> u32 {
        self.until.saturating_sub(now)
    }
}

state_hash!(BackpackHistory {
    seen,
    holder,
    until
});

/// One item in a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    pub id: ItemId,
    /// Level of this physical item, preserved by moves.
    pub level: u8,
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
    pub backpack: BackpackHistory,
    /// Whether the physical item's toggled effects are enabled.
    pub active: bool,
    /// Physical recovery of a breakable passive; cooldown refresh does not repair it.
    pub passive_wait: u32,
}

impl ItemStack {
    /// An untouched purchase with the catalog's initial charges and mode.
    /// Nothing for an item no shop sells.
    pub fn bought(id: ItemId, owner: SlotId, tick: u32) -> Option<Self> {
        let def = item_def(id).filter(|def| !def.unbuyable)?;
        Some(Self {
            id,
            level: 1,
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
            backpack: BackpackHistory::default(),
            active: true,
            passive_wait: 0,
        })
    }

    pub fn dropped(id: ItemId) -> Option<Self> {
        let def = item_def(id)?;
        Some(Self {
            mode: def.mode,
            life: def.lifetime,
            ..Self::granted(id, SlotId(u8::MAX), 0, def.charges)
        })
    }

    /// `charges` of an item that comes for nothing, as a lotus does: touched, so
    /// that no purchase window refunds it.
    pub fn granted(id: ItemId, owner: SlotId, tick: u32, charges: u8) -> Self {
        Self {
            id,
            level: 1,
            charges,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: tick,
            touched: true,
            owner,
            for_sale: false,
            free_charges: 0,
            sentries: 0,
            sentry_active: false,
            locked: false,
            disassemble_until: 0,
            life: item_def(id).map_or(0, |def| def.lifetime.max(def.pickup_lifetime)),
            rune: None,
            backpack: BackpackHistory::default(),
            active: true,
            passive_wait: 0,
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
        if self.id != merged.id {
            self.backpack.inherit_build(source.backpack);
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
        self.backpack.inherit(source.backpack);
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
        self.shelf().receiving_slot(incoming).filter(|at| {
            item_def(incoming.id)
                .is_none_or(|def| def.limits.backpack || *at < rules::INVENTORY_SLOTS)
        })
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
    /// stack brought out of the backpack waits [`rules::BACKPACK_MUTE_TICKS`]
    /// before granting bonuses or being used. This bag-only entry assumes
    /// no shop exemption; the world supplies that context in `move_item`.
    /// False when nothing moved.
    pub fn swap_slots(&mut self, from: usize, to: usize) -> bool {
        self.swap_slots_with_backpack_mute(from, to, rules::BACKPACK_MUTE_TICKS, 0)
    }

    /// The world supplies zero beside either shop, clearing any
    /// residual inactivity on a stack landing from the backpack.
    pub(crate) fn swap_slots_with_backpack_mute(
        &mut self,
        from: usize,
        to: usize,
        backpack_mute: u32,
        now: u32,
    ) -> bool {
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
        let mute = |stack: &mut ItemStack, origin: usize, landed: usize, grace: bool| {
            stack.touched = true;
            if in_backpack(origin) && in_inventory(landed) {
                stack.mute = if backpack_mute == 0 {
                    0
                } else if grace {
                    stack.mute
                } else {
                    stack.mute.max(backpack_mute)
                };
                stack.backpack.leave();
            }
        };
        let moved_grace = moved.backpack.left(now) > 0;
        if let Some(mut target) = displaced
            && target.merge_from(&mut moved, !self.stash)
        {
            moved.touched = true;
            mute(&mut target, from, to, moved_grace);
            *self.slot_mut(to).expect("a slot") = Some(target);
            *self.slot_mut(from).expect("a slot") = (moved.charges > 0).then_some(moved);
            return true;
        }
        let mut displaced = displaced;
        mute(&mut moved, from, to, moved_grace);
        if let Some(stack) = displaced.as_mut() {
            let grace = stack.backpack.left(now) > 0;
            mute(stack, to, from, grace);
        }
        *self.slot_mut(to).expect("a slot") = Some(moved);
        *self.slot_mut(from).expect("a slot") = displaced;
        true
    }

    /// Runs the life of every stack that has one a tick down, and clears the slot of one that
    /// has run out.
    pub fn lapse(&mut self) {
        self.lapse_with(|_| {});
    }

    pub fn lapse_with(&mut self, mut expired: impl FnMut(ItemStack)) {
        let mut run = |slot: &mut Option<ItemStack>| {
            if let Some(stack) = slot {
                stack.passive_wait = stack.passive_wait.saturating_sub(1);
                if item_def(stack.id).is_some_and(|def| def.attack_break.is_some()) {
                    stack.cooldown = stack.cooldown.min(stack.passive_wait);
                }
            }
            if let Some(stack) = slot
                && stack.life > 0
            {
                stack.life -= 1;
                if stack.life == 0 {
                    expired(*stack);
                    *slot = None;
                }
            }
        };
        self.slots.iter_mut().for_each(&mut run);
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

pub use bota_core::{BAG_SLOTS, in_backpack, in_inventory, in_stash, in_town_portal};

impl From<ItemStack> for bota_core::OrderItemFacts {
    fn from(stack: ItemStack) -> Self {
        Self {
            id: stack.id,
            owner: stack.owner,
            rune: stack.rune.is_some(),
            free_charges: stack.free_charges,
        }
    }
}

state_hash!(ItemStack {
    id,
    level,
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
    rune,
    backpack,
}; 0xB4 => { active: true, passive_wait: 0 });
state_hash!(Inventory {
    slots,
    town_portal,
    stash
});
