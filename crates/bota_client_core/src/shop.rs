//! What a seat pays for an item and what it still lacks to build one, over
//! the core's item table.

use bota_core::{hero_def, item_def, missing_parts};
use bota_proto::{Attribute, ItemId, PlayerView, WorldView};

/// Every item in a seat's hero's bag and in its stash that the seat bought
/// and has not marked for sale, for [`price_for`].
pub fn held_items(view: &WorldView, p: &PlayerView) -> Vec<ItemId> {
    // While the hero lies dead what it carried counts, as the server buys.
    let bag = p
        .unit
        .and_then(|id| view.unit(id))
        .map(|u| u.items.as_slice())
        .or_else(|| p.kit.as_ref().map(|kit| kit.items.as_slice()))
        .unwrap_or_default();
    bag.iter()
        .chain(p.stash.iter().flatten())
        .flatten()
        .filter(|item| item.owner == p.slot && item.selling != Some(true))
        .map(|item| item.id)
        .collect()
}

/// What the shop asks for one item whole, before anything already in hand.
pub fn whole_price(item: ItemId) -> i32 {
    item_def(item).map_or(0, |def| def.cost)
}

/// The primary attribute of a seat's hero, which picks among an item's
/// recipes of one price.
pub fn primary_of(p: &PlayerView) -> Option<Attribute> {
    hero_def(p.hero).and_then(|hero| hero.unit.primary)
}

/// What the shop asks a seat holding `held` for one item: an item without
/// parts costs its whole price however many are held; a built one costs the
/// parts not found in `held`, each spent once, of its cheapest recipe.
pub fn price_for(item: ItemId, held: &[ItemId], primary: Option<Attribute>) -> i32 {
    missing_parts(item, held, primary)
        .iter()
        .map(|part| whole_price(*part))
        .sum()
}
