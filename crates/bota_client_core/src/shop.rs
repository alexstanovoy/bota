//! The shop as a client reads it: what is sold, what a seat pays for an item and what it still
//! lacks to build one, and whether a purchase fits, all by bota_core's rules, the ones the
//! server buys by.

use bota_core::{
    Berth, PurchaseCarrier, PurchaseContext, Shelf, hero_def, missing_parts, place_shop_purchase,
    purchase_pool, shop_access, sold,
};
use bota_proto::{Attribute, ItemId, PlayerView, RejectReason, UnitKind, UnitView, WorldView};

use crate::MatchStatic;

/// Every item the shop of a match on `catalogue` sells, in id order: neither retired nor
/// unbuyable, and in the catalogue ([`MatchStatic::catalogue`](crate::MatchStatic::catalogue)).
pub fn sold_items(catalogue: bota_core::Catalogue) -> impl Iterator<Item = ItemId> {
    catalogue.sold_items()
}

/// Every item that counts toward a seat's purchase, for [`price_for`]: the seat's own stacks,
/// not marked for sale, in its hero's bag (the kit it carried while it lies dead), its stash and
/// its courier, as the server buys. As in Dota, a purchase counts everything the player owns.
pub fn held_items(view: &WorldView, p: &PlayerView) -> Vec<ItemId> {
    let [hero, courier] = carriers(view, p);
    let bag = hero
        .map(|u| u.items.as_slice())
        .or_else(|| p.kit.as_ref().map(|kit| kit.items.as_slice()))
        .unwrap_or_default();
    let courier = courier
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
/// Base purchases reach the hero at either base or during normal respawn, then overflow
/// into the stash; secret-shop parts require a physical
/// carrier there. Mergeable charges fill a stack first. A seat whose stash is not told
/// (the other side's) has none to ask about.
pub fn purchase_fits(
    view: &WorldView,
    p: &PlayerView,
    statics: &MatchStatic,
    item: ItemId,
) -> bool {
    purchase_placement(view, p, statics, item).is_ok()
}

pub(crate) fn purchase_placement(
    view: &WorldView,
    p: &PlayerView,
    statics: &MatchStatic,
    item: ItemId,
) -> Result<Vec<Berth>, RejectReason> {
    if p.stash.is_none() {
        return Err(RejectReason::InventoryFull);
    }
    let parts = missing_for(item, &held_items(view, p), primary_of(p));
    let context = purchase_context(view, p, statics);
    place_shop_purchase(p.slot, &parts, &context)
}

/// The buyer's receiving inventory, living courier and storage, with the announced shop access.
/// An untold stash has no known space, but physical access remains available to indicators.
pub fn purchase_context<'a>(
    view: &'a WorldView,
    player: &'a PlayerView,
    statics: &MatchStatic,
) -> PurchaseContext<'a, bota_proto::ItemView> {
    purchase_context_on(view, player, statics, statics.metadata().universal_shop)
}

pub(crate) fn purchase_context_on<'a>(
    view: &'a WorldView,
    player: &'a PlayerView,
    geometry: &impl bota_core::ShopGeometry,
    universal_shop: bool,
) -> PurchaseContext<'a, bota_proto::ItemView> {
    let [hero, courier] = carriers(view, player);
    PurchaseContext {
        hero: if player.respawn_left > 0 {
            player.kit.as_ref().map(|kit| PurchaseCarrier {
                bag: Shelf {
                    slots: &kit.items,
                    town_portal: Some(&kit.town_portal),
                    combines: true,
                },
                shops: Default::default(),
            })
        } else {
            hero.and_then(|unit| purchase_carrier(unit, geometry))
        },
        courier: courier.and_then(|unit| purchase_carrier(unit, geometry)),
        stash: Shelf {
            slots: player.stash.as_deref().unwrap_or_default(),
            town_portal: None,
            combines: false,
        },
        universal_shop,
        respawning: player.respawn_left > 0
            && player.kit.as_ref().is_some_and(|kit| !kit.reincarnating),
    }
}

fn carriers<'a>(view: &'a WorldView, player: &PlayerView) -> [Option<&'a UnitView>; 2] {
    [
        player.unit.and_then(|id| view.unit(id)),
        view.units
            .iter()
            .find(|unit| unit.kind == UnitKind::Courier && unit.owner == Some(player.slot)),
    ]
}

pub(crate) fn purchase_carrier<'a>(
    unit: &'a UnitView,
    geometry: &impl bota_core::ShopGeometry,
) -> Option<PurchaseCarrier<'a, bota_proto::ItemView>> {
    if unit.hp <= 0 {
        return None;
    }
    Some(PurchaseCarrier {
        bag: Shelf {
            slots: unit.items.as_slice(),
            town_portal: (unit.kind == UnitKind::Hero).then_some(&unit.town_portal),
            combines: true,
        },
        shops: shop_access(geometry, unit.pos, unit.bound),
    })
}
