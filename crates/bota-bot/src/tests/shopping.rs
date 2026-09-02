//! What the bot buys, and what it reckons its goods are worth.

use bota_proto::{ItemId, ItemView, SlotId, UnitKind};

use crate::{Field, ITEMS_SOLD, Role, SHOPPING, cost_of, how_many_held, next_to_buy};

/// A tick where this seat's hero carries the items named, with the gold given.
fn holding(items: &[u16], gold: i32) -> bota_proto::WorldView {
    crate::tests::a_tick_holding(items, gold)
}

/// The field of a tick, for a seat in the middle.
fn field_of(view: &bota_proto::WorldView) -> Field<'_> {
    Field::of(view, SlotId(0), Role::Mid).expect("the seat is in the tick")
}

#[test]
fn every_item_the_shop_sells_has_a_price() {
    // Left short, an item missing from the table is worth nothing at all to
    // the reckoning, and the whole of the last lesson is net worth.
    for item in 0..ITEMS_SOLD as u16 {
        assert!(
            cost_of(item) > 0,
            "item {item} is sold and must have a price"
        );
    }
    assert_eq!(
        cost_of(ITEMS_SOLD as u16),
        0,
        "and a number the shop does not sell costs nothing"
    );
}

#[test]
fn a_few_prices_are_what_the_shop_charges() {
    // Spot checks across the table, so that a row inserted in the middle shows
    // up here rather than as a net worth quietly counted wrong.
    assert_eq!(cost_of(crate::TANGO), 90);
    assert_eq!(cost_of(crate::BOOTS), 500);
    assert_eq!(cost_of(crate::CIRCLET), 155);
    assert_eq!(cost_of(crate::TREADS), 1400);
    assert_eq!(cost_of(crate::BLINK), 2250);
    assert_eq!(cost_of(crate::RECIPE_MAGIC_WAND), 150);
}

#[test]
fn what_the_goods_are_worth_counts_the_costly_ones() {
    let bare = holding(&[], 600);
    assert_eq!(crate::worth_of_goods(&field_of(&bare)), 0);
    let rich = holding(&[crate::BLINK, crate::TREADS], 0);
    assert_eq!(
        crate::worth_of_goods(&field_of(&rich)),
        2250 + 1400,
        "a dagger and a pair of treads are not an empty bag"
    );
}

#[test]
fn a_part_that_went_into_a_build_still_counts_as_bought() {
    // The trap the whole build order rests on. Counted only where it can be
    // seen, the boots that became Power Treads look unbought, and the bot buys
    // another pair every time it can afford one.
    let treads = holding(&[crate::TREADS], 5000);
    let field = field_of(&treads);
    for part in [crate::BOOTS, crate::GLOVES, crate::BELT] {
        assert_eq!(
            how_many_held(&field, part),
            1,
            "a part of the treads is a part it paid for"
        );
    }
    assert_ne!(
        next_to_buy(&field),
        Some(ItemId(crate::BOOTS)),
        "so it does not buy boots it is already wearing"
    );
}

#[test]
fn a_build_of_two_of_a_part_counts_both() {
    let wand = holding(&[crate::MAGIC_WAND], 0);
    assert_eq!(
        how_many_held(&field_of(&wand), crate::BRANCH),
        2,
        "a wand ate two branches"
    );
}

#[test]
fn the_list_is_bought_in_order_and_only_once() {
    // Nothing on the list is bought twice, and nothing further down is reached
    // before what is above it.
    let mut bag: Vec<u16> = Vec::new();
    let mut bought: Vec<u16> = Vec::new();
    for _ in 0..SHOPPING.len() {
        let view = holding(&bag, 9000);
        let Some(next) = next_to_buy(&field_of(&view)) else {
            break;
        };
        bought.push(next.0);
        bag.push(next.0);
    }
    assert_eq!(
        bought,
        SHOPPING.to_vec(),
        "with gold enough it buys the list, in order, once each"
    );
    let full = holding(&bag, 9000);
    assert_eq!(
        next_to_buy(&field_of(&full)),
        None,
        "and then it wants nothing"
    );
}

#[test]
fn nothing_is_bought_that_cannot_be_paid_for() {
    let view = holding(&[], 60);
    let field = field_of(&view);
    let next = next_to_buy(&field).expect("sixty buys a branch");
    assert!(
        cost_of(next.0) <= 60,
        "it does not ask for what it cannot pay for"
    );
    let broke = holding(&[], 0);
    assert_eq!(
        next_to_buy(&field_of(&broke)),
        None,
        "and nothing with none"
    );
}

#[test]
fn what_waits_in_the_stash_or_rides_the_courier_counts_as_owned() {
    // Buying a second of something already on its way is how gold gets wasted.
    let mut view = holding(&[], 5000);
    let carried = Some(ItemView {
        id: ItemId(crate::TANGO),
        charges: Some(3),
        cooldown_left: 0,
        mute_left: 0,
        mana_cost: 0,
        range: 0,
        aim: None,
        mode: None,
        for_sale: false,
    });
    for player in &mut view.players {
        if player.slot == SlotId(0) {
            player.stash = Some(vec![carried, None, None, None, None, None]);
        }
    }
    for body in &mut view.units {
        if body.kind == UnitKind::Courier {
            body.items[0] = Some(ItemView {
                id: ItemId(crate::QUELLING),
                charges: Some(0),
                cooldown_left: 0,
                mute_left: 0,
                mana_cost: 0,
                range: 0,
                aim: None,
                mode: None,
                for_sale: false,
            });
        }
    }
    let field = field_of(&view);
    assert_eq!(
        how_many_held(&field, crate::TANGO),
        1,
        "the stash holds one"
    );
    assert_eq!(
        how_many_held(&field, crate::QUELLING),
        1,
        "and the courier is bringing the other"
    );
}
