//! What an entity carries.

use bota_proto::{Attribute, ItemId, SlotId};

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
