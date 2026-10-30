//! The shop as a client reads it: what is sold, what a seat pays for an item and what it still
//! lacks to build one, and whether a purchase fits, all by bota_core's rules, the ones the
//! server buys by.

use bota_core::{
    MapDef, Shelf, at_home_shop, hero_def, missing_parts, place_purchase, purchase_pool, sold,
};
use bota_proto::{Attribute, ItemId, PlayerView, UnitKind, WorldView};

/// Every item the shop sells, in id order: neither retired nor unbuyable.
pub fn sold_items() -> impl Iterator<Item = ItemId> {
    bota_core::sold_items()
}

/// Every item that counts toward a seat's purchase, for [`price_for`]: the seat's own stacks,
/// not marked for sale, in its hero's bag (the kit it carried while it lies dead), its stash and
/// its courier, as the server buys. As in Dota, a purchase counts everything the player owns.
pub fn held_items(view: &WorldView, p: &PlayerView) -> Vec<ItemId> {
    let bag = p
        .unit
        .and_then(|id| view.unit(id))
        .map(|u| u.items.as_slice())
        .or_else(|| p.kit.as_ref().map(|kit| kit.items.as_slice()))
        .unwrap_or_default();
    let courier = view
        .units
        .iter()
        .find(|unit| unit.kind == UnitKind::Courier && unit.owner == Some(p.slot))
        .map(|unit| unit.items.as_slice())
        .unwrap_or_default();
    purchase_pool(
        p.slot,
        bag.iter()
            .chain(p.stash.iter().flatten())
            .chain(courier)
            .flatten(),
    )
}

/// What the shop asks for one item whole, before anything already in hand; nothing for an item it
/// does not sell.
pub fn whole_price(item: ItemId) -> Option<i32> {
    sold(item).map(|def| def.cost)
}

/// The primary attribute of a seat's hero, which picks among an item's recipes of one price.
pub fn primary_of(p: &PlayerView) -> Option<Attribute> {
    hero_def(p.hero).and_then(|hero| hero.unit.primary)
}

/// The parts of `item` a seat holding `held` still has to buy: its cheapest recipe, the one set
/// to `primary` first among equals ([`missing_parts`]).
pub fn missing_for(item: ItemId, held: &[ItemId], primary: Option<Attribute>) -> Vec<ItemId> {
    missing_parts(item, held, primary)
}

/// What the shop asks a seat holding `held` for one item: an item without parts costs its whole
/// price however many are held; a built one costs the parts not found in `held`, each spent
/// once, of its cheapest recipe.
pub fn price_for(item: ItemId, held: &[ItemId], primary: Option<Attribute>) -> i32 {
    missing_for(item, held, primary)
        .into_iter()
        .filter_map(whole_price)
        .sum()
}

/// Whether the server finds room for what buying `item` still takes, whatever the seat can pay:
/// its parts go to the hero's bag while the hero stands in its shop and to the stash otherwise,
/// mergeable charges filling a stack first ([`place_purchase`]). A seat whose stash is not told
/// (the other side's) has none to ask about.
pub fn purchase_fits(view: &WorldView, p: &PlayerView, map: &MapDef, item: ItemId) -> bool {
    let Some(stash) = p.stash.as_deref() else {
        return false;
    };
    let hero = p.unit.and_then(|id| view.unit(id));
    let parts = missing_for(item, &held_items(view, p), primary_of(p));
    let bag = hero
        .filter(|hero| at_home_shop(map, hero.team, hero.pos))
        .map(|hero| Shelf {
            slots: hero.items.as_slice(),
            town_portal: Some(&hero.town_portal),
            combines: true,
        });
    let stash = Shelf {
        slots: stash,
        town_portal: None,
        combines: false,
    };
    place_purchase(p.slot, &parts, bag.as_ref(), &stash).is_some()
}
