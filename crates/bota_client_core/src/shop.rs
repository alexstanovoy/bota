//! What a seat pays for an item and what it still lacks to build one, over
//! the core's item table.

use bota_core::item_def;
use bota_proto::{ItemId, PlayerView, WorldView};

/// Every item in a seat's hero's bag and in its stash that the seat bought
/// and has not marked for sale, for [`price_for`].
pub fn held_items(view: &WorldView, p: &PlayerView) -> Vec<ItemId> {
    let bag = p
        .unit
        .and_then(|id| view.units.iter().find(|u| u.id == id))
        .map(|u| u.items.as_slice())
        .unwrap_or_default();
    bag.iter()
        .chain(p.stash.iter().flatten())
        .flatten()
        .filter(|item| item.owner == p.slot && !item.for_sale)
        .map(|item| item.id)
        .collect()
}

/// What one item is built from; nothing for one bought whole.
fn components_of(item: ItemId) -> &'static [ItemId] {
    item_def(item).map_or(&[], |def| def.components)
}

/// What the shop asks for one item whole, before anything already in hand.
pub fn whole_price(item: ItemId) -> i32 {
    item_def(item).map_or(0, |def| def.cost)
}

/// What the shop asks a seat holding `held` for one item: an item without
/// parts costs its whole price however many are held; a built one costs the
/// parts not found in `held`, each spent once.
pub fn price_for(item: ItemId, held: &[ItemId]) -> i32 {
    missing_parts(item, held)
        .iter()
        .map(|part| whole_price(*part))
        .sum()
}

/// The parts bought whole that a seat holding `held` still lacks for one
/// item, in build order: the item itself when it has no parts.
pub fn missing_parts(item: ItemId, held: &[ItemId]) -> Vec<ItemId> {
    bota_core::missing_parts(item, held, components_of)
}
