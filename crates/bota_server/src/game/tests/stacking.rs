//! Which items stack, as Dota 7.41f's `items.txt` has it (`ItemStackable`,
//! `ItemStackableMax`, `ItemInitialCharges`): a second purchase of one that
//! stacks joins the first in its slot, one that does not takes a slot of its
//! own.

use bota_core::{ITEMS, ItemDef};
use bota_proto::ItemId;

use super::support::*;

/// The items the file marks `ItemStackable` 1, the Town Portal Scroll aside
/// (it stacks in its own slot).
const STACKING: [ItemId; 7] = [
    ItemId::CLARITY,
    ItemId::HEALING_SALVE,
    ItemId::TANGO,
    ItemId::MANGO,
    ItemId::BLOOD_GRENADE,
    ItemId::OBSERVER_WARD,
    ItemId::SENTRY_WARD,
];

fn bought_whole(def: &ItemDef) -> bool {
    def.components.is_empty() && !def.unbuyable && !def.retired && def.name != "Town Portal Scroll"
}

#[test]
fn a_second_purchase_stacks_exactly_the_items_dota_stacks() {
    for (index, def) in ITEMS
        .iter()
        .enumerate()
        .filter(|(_, def)| bought_whole(def))
    {
        let id = ItemId(index as u16);
        let (mut world, hero) = a_hero_with_gold(100_000);
        for _ in 0..2 {
            assert!(
                world.buy(bota_proto::SlotId(0), id, &mut Vec::new()),
                "{}",
                def.name
            );
        }
        let slots = world.inventory.get(hero).expect("bag").held().count();
        let expected = if STACKING.contains(&id) { 1 } else { 2 };
        assert_eq!(slots, expected, "{} takes {expected} slot(s)", def.name);
    }
}

#[test]
fn consumables_add_up_their_charges_and_sell_at_the_price_of_each() {
    for (item, per_purchase) in [
        (ItemId::CLARITY, 1u8),
        (ItemId::HEALING_SALVE, 1),
        (ItemId::TANGO, 3),
    ] {
        let (mut world, hero) = a_hero_with_gold(100_000);
        for _ in 0..3 {
            assert!(world.buy(bota_proto::SlotId(0), item, &mut Vec::new()));
        }
        let stack = slot_of(&world, hero, 0).expect("one stack");
        assert_eq!(stack.charges, 3 * per_purchase);
        let gold = world.seats[0].gold;
        assert!(world.sell_item(bota_proto::SlotId(0), hero, 0));
        assert_eq!(
            world.seats[0].gold - gold,
            3 * price_of(item),
            "a fresh stack refunds each purchase in full"
        );
    }
}

#[test]
fn a_stack_with_a_cap_overflows_into_a_slot_of_its_own() {
    // `ItemStackableMax`: a Mango stack holds 3 (a Blood Grenade's 2 is out of
    // reach of the shop's stock of 2 at the start).
    let (mut world, hero) = a_hero_with_gold(100_000);
    for _ in 0..4 {
        assert!(world.buy(bota_proto::SlotId(0), ItemId::MANGO, &mut Vec::new()));
    }
    assert_eq!(slot_of(&world, hero, 0).map(|s| s.charges), Some(3));
    assert_eq!(slot_of(&world, hero, 1).map(|s| s.charges), Some(1));
}
