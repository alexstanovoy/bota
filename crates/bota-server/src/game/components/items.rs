//! What an entity carries.

use bota_proto::{Attribute, ItemId, SlotId};

use crate::game::item_def;

/// One item in a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    /// What it is.
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
}

impl ItemStack {
    /// An untouched purchase with the catalog's initial charges and mode.
    pub fn bought(id: ItemId, owner: SlotId, tick: u32) -> Option<Self> {
        let def = item_def(id)?;
        if def.stack_limit > 0 {
            assert_eq!(def.charges, 1);
            assert!(def.charges <= def.stack_limit);
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
        })
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
        let before = u16::from(self.charges) + u16::from(source.charges);
        self.charges += moved;
        source.charges -= moved;
        self.bought_tick = self.bought_tick.min(source.bought_tick);
        self.cooldown = self.cooldown.max(source.cooldown);
        self.mute = self.mute.max(source.mute);
        self.touched |= source.touched;
        assert_eq!(self.owner, source.owner);
        assert_eq!(u16::from(self.charges) + u16::from(source.charges), before);
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
}

impl Inventory {
    /// An inventory of that many empty slots.
    pub fn empty(slots: usize) -> Inventory {
        Inventory {
            slots: vec![None; slots],
        }
    }

    /// Every item held, in slot order.
    pub fn held(&self) -> impl Iterator<Item = &ItemStack> {
        self.slots.iter().flatten()
    }

    /// First compatible stack fitting the whole arrival, then the first empty slot.
    pub fn receiving_slot(&self, incoming: &ItemStack) -> Option<usize> {
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
        assert!(at < self.slots.len());
        if let Some(held) = self.slots[at].as_mut() {
            assert!(held.merge_from(&mut incoming));
            assert_eq!(incoming.charges, 0);
        } else {
            self.slots[at] = Some(incoming);
        }
        true
    }
}

/// Slots a hero carries on itself: the inventory proper and the backpack.
pub const BAG_SLOTS: usize =
    crate::game::rules::INVENTORY_SLOTS + crate::game::rules::BACKPACK_SLOTS;

/// Whether a slot number is one of the inventory proper, where items work.
pub fn in_inventory(slot: usize) -> bool {
    slot < crate::game::rules::INVENTORY_SLOTS
}

/// Whether a slot number is one of the backpack, where they are carried inert.
pub fn in_backpack(slot: usize) -> bool {
    (crate::game::rules::INVENTORY_SLOTS..BAG_SLOTS).contains(&slot)
}

/// Whether a slot number is one of the stash at the shop.
pub fn in_stash(slot: usize) -> bool {
    slot >= BAG_SLOTS
}
