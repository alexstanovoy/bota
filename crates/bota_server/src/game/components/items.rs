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
}

impl ItemStack {
    /// An untouched purchase with the catalog's initial charges and mode.
    pub fn bought(id: ItemId, owner: SlotId, tick: u32) -> Option<Self> {
        let def = item_def(id)?;
        if def.stack_limit > 0 {
            assert_eq!(def.charges, 1);
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

    /// Charges another stack may add; ownership, mode, and sale marks must match.
    pub fn merge_room(&self, source: &Self) -> u8 {
        if self.id != source.id
            || self.owner != source.owner
            || self.mode != source.mode
            || self.for_sale != source.for_sale
            || self.charges == 0
            || source.charges == 0
        {
            return 0;
        }
        let limit = item_def(self.id).map_or(0, |def| def.stack_limit);
        if limit == 0 {
            return 0;
        }
        assert!(self.charges <= limit);
        assert!(source.charges <= limit);
        limit - self.charges
    }

    /// Merges up to the cap, retaining the oldest purchase and strongest restrictions.
    /// A source reduced to zero must be removed from its slot by the caller.
    pub fn merge_from(&mut self, source: &mut Self) -> bool {
        let moved = self.merge_room(source).min(source.charges);
        if moved == 0 {
            return false;
        }
        self.charges += moved;
        source.charges -= moved;
        self.bought_tick = self.bought_tick.min(source.bought_tick);
        self.cooldown = self.cooldown.max(source.cooldown);
        self.mute = self.mute.max(source.mute);
        self.touched |= source.touched;
        // Free charges travel with the charges that carry them, and stay
        // within the charges either side holds.
        let free = moved.min(source.free_charges);
        self.free_charges += free;
        source.free_charges -= free;
        assert!(self.free_charges <= self.charges);
        assert!(source.free_charges <= source.charges);
        true
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
}

impl Inventory {
    /// An inventory of that many empty slots and no town portal slot.
    pub fn empty(slots: usize) -> Inventory {
        Inventory {
            slots: vec![None; slots],
            town_portal: None,
        }
    }

    /// A hero's bag: the inventory, the backpack and the town portal slot,
    /// that one holding the scroll every hero starts with.
    pub fn of_hero(owner: SlotId, tick: u32) -> Inventory {
        Inventory {
            slots: vec![None; BAG_SLOTS],
            town_portal: Some(Some(ItemStack::free_scroll(owner, tick))),
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
            let fits = held.is_none_or(|held| held.merge_room(incoming) >= incoming.charges);
            return fits.then_some(rules::TOWN_PORTAL_SLOT);
        }
        let merge = self.slots.iter().position(|held| {
            held.is_some_and(|held| {
                incoming.charges > 0 && held.merge_room(incoming) >= incoming.charges
            })
        });
        merge.or_else(|| self.slots.iter().position(Option::is_none))
    }

    /// Receives one whole stack or changes nothing; merges only when the entire stack fits.
    pub fn receive(&mut self, mut incoming: ItemStack) -> bool {
        let Some(at) = self.receiving_slot(&incoming) else {
            return false;
        };
        let slot = self.slot_mut(at).expect("a receiving slot exists");
        if let Some(held) = slot.as_mut() {
            assert!(held.merge_from(&mut incoming));
            assert_eq!(incoming.charges, 0);
        } else {
            *slot = Some(incoming);
        }
        true
    }

    /// Takes a stack handed over whole: a scroll into the town portal slot
    /// when the bag has one, anything else into the first empty slot.
    pub fn stow(&mut self, incoming: ItemStack) -> bool {
        if incoming.is_scroll() && self.town_portal.is_some() {
            return self.receive(incoming);
        }
        let Some(free) = self.slots.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *free = Some(incoming);
        true
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
