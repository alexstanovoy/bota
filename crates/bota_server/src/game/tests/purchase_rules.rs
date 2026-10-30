//! One shop: the client asks the rules the server buys by, so what it lists is what the server
//! sells and what it says fits is what the server accepts.

use std::collections::BTreeSet;

use bota_client_core::{ClientSession, purchase_fits, sold_items};
use bota_core::ITEMS;
use bota_proto::{ItemId, Order, SlotId, Vec2};

use super::client::seated;
use super::support::fresh_stack;
use crate::game::{ItemStack, World};
use crate::local::LocalMatch;

const BUYER: SlotId = SlotId(0);

/// Fills every slot of a seat 0 container with a stack of `item`.
fn full_of(slots: &mut [Option<ItemStack>], item: ItemId) {
    for held in slots {
        *held = Some(fresh_stack(item, 1));
    }
}

/// A state of the bag, the stash and the hero's whereabouts a purchase meets.
struct State {
    name: &'static str,
    home: bool,
    bag: Option<ItemId>,
    stash: Option<ItemId>,
}

const STATES: [State; 9] = [
    State {
        name: "everything free, at the shop",
        home: true,
        bag: None,
        stash: None,
    },
    State {
        name: "full bag, free stash",
        home: true,
        bag: Some(ItemId::QUELLING_BLADE),
        stash: None,
    },
    State {
        name: "full bag, full stash",
        home: true,
        bag: Some(ItemId::QUELLING_BLADE),
        stash: Some(ItemId::QUELLING_BLADE),
    },
    State {
        name: "away, free stash",
        home: false,
        bag: None,
        stash: None,
    },
    State {
        name: "away, full stash",
        home: false,
        bag: None,
        stash: Some(ItemId::QUELLING_BLADE),
    },
    State {
        name: "stacks with room everywhere",
        home: true,
        bag: Some(ItemId::HEALING_SALVE),
        stash: Some(ItemId::HEALING_SALVE),
    },
    State {
        name: "wards that could combine, full stash",
        home: true,
        bag: Some(ItemId::OBSERVER_WARD),
        stash: Some(ItemId::QUELLING_BLADE),
    },
    State {
        name: "away, a stash of wards that cannot combine",
        home: false,
        bag: None,
        stash: Some(ItemId::OBSERVER_WARD),
    },
    State {
        name: "away, a stash of salves",
        home: false,
        bag: None,
        stash: Some(ItemId::HEALING_SALVE),
    },
];

/// Puts the match in `state` and lets the session see it.
fn arranged(state: &State) -> (LocalMatch, ClientSession) {
    let (mut local, mut session) = seated();
    let world = local.world_mut();
    let hero = world.seats[0].unit.expect("a hero");
    world.seats[0].gold = 1_000_000;
    if let Some(item) = state.bag {
        full_of(
            &mut world.inventory.get_mut(hero).expect("a bag").slots,
            item,
        );
    }
    if let Some(item) = state.stash {
        full_of(&mut world.seats[0].stash.slots, item);
    }
    if !state.home {
        world.transform.get_mut(hero).expect("placed").pos = Vec2::from_ints(9_000, 9_000);
    }
    assert_eq!(world.at_shop(hero), state.home, "{}", state.name);
    let step = local.step(&[&[], &[]]).expect("a step");
    for message in step.into_iter().next().expect("seat 0") {
        session.ingest(message, &mut ()).expect("folds");
    }
    (local, session)
}

/// Whether the server takes a buy order of `item` from seat 0 in `world`: the order passes the
/// checks every order does, and the purchase itself goes through.
fn server_accepts(world: &World, item: ItemId) -> bool {
    let order = Order::Buy { item };
    let allowed = world.check_order(BUYER, None, &order, false).is_ok();
    let bought = world.clone().buy(BUYER, item, &mut Vec::new());
    assert_eq!(
        allowed, bought,
        "{item:?}: the order check and the purchase agree"
    );
    allowed
}

#[test]
fn what_the_client_says_fits_is_what_the_server_accepts_for_every_item_in_every_state() {
    for state in &STATES {
        let (local, session) = arranged(state);
        let view = session.current().expect("a view");
        let player = view.player(BUYER).expect("the buyer");
        let mut differ = Vec::new();
        for id in 0..ITEMS.len() as u16 {
            let item = ItemId(id);
            let fits = purchase_fits(view, player, session.map(), item);
            if fits != server_accepts(local.world(), item) {
                differ.push((id, fits));
            }
        }
        assert!(
            differ.is_empty(),
            "{}: the client and the server differ on {differ:?}",
            state.name
        );
    }
}

#[test]
fn the_items_a_client_lists_are_exactly_the_items_the_server_sells() {
    let (local, session) = arranged(&STATES[0]);
    let listed: BTreeSet<u16> = sold_items().map(|item| item.0).collect();
    let accepted: BTreeSet<u16> = (0..ITEMS.len() as u16)
        .filter(|id| server_accepts(local.world(), ItemId(*id)))
        .collect();
    assert_eq!(listed, accepted);
    // And the wire's statement of the shop says the same.
    let wire: BTreeSet<u16> = session
        .statics()
        .shop()
        .iter()
        .map(|entry| entry.id.0)
        .collect();
    assert_eq!(wire, listed);
    assert!(
        !listed.contains(&ItemId::WARD_DISPENSER.0),
        "a dispenser is made, not sold"
    );
}

/// The states tell something: a purchase that needs room is refused where none is, and the town
/// portal slot is room for a scroll whatever the bag holds.
#[test]
fn a_full_bag_and_stash_refuse_a_purchase_but_a_scroll_a_salve_and_a_ward_find_room() {
    let (local, session) = arranged(&STATES[2]);
    let view = session.current().expect("a view");
    let player = view.player(BUYER).expect("the buyer");
    let fits = |item| purchase_fits(view, player, session.map(), item);
    assert!(!fits(ItemId::BOOTS));
    assert!(
        fits(ItemId::TOWN_PORTAL_SCROLL),
        "a scroll joins the one in its own slot"
    );
    assert!(!fits(ItemId::HEALING_SALVE));
    assert!(server_accepts(local.world(), ItemId::TOWN_PORTAL_SCROLL));
    assert!(!server_accepts(local.world(), ItemId::BOOTS));
    let (_, session) = arranged(&STATES[5]);
    let view = session.current().expect("a view");
    let player = view.player(BUYER).expect("the buyer");
    assert!(purchase_fits(
        view,
        player,
        session.map(),
        ItemId::HEALING_SALVE
    ));
    assert!(!purchase_fits(view, player, session.map(), ItemId::CLARITY));
}
