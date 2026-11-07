//! One catalogue across both shops: the client lists what is sold, while physical access and
//! storage decide which purchases the server accepts.

use std::collections::BTreeSet;

use bota_client_core::{ClientSession, purchase_fits, sold_items};
use bota_core::ITEMS;
use bota_proto::{ItemId, Order, SlotId, Vec2};

use super::client::seated;
use super::support::{every_shelf_full, fresh_stack};
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
    // What fits is a question of room: every shelf holds what it can, one that opens later too.
    every_shelf_full(world);
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
            let fits = purchase_fits(view, player, session.statics(), item);
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
    let (mut local, session) = arranged(&STATES[0]);
    let listed: BTreeSet<u16> = sold_items(bota_core::Catalogue::FULL)
        .map(|item| item.0)
        .collect();
    assert!(listed.contains(&ItemId::DEMON_EDGE.0));
    assert!(
        !server_accepts(local.world(), ItemId::DEMON_EDGE),
        "the catalogue includes secret components unavailable at the base shop"
    );
    // Leave the hero at base and put the personal courier in a secret-shop trigger:
    // every listed part is now physically reachable, including mixed-shop recipes.
    let world = local.world_mut();
    let courier = world.seats[0].courier.expect("personal courier");
    world.transform.get_mut(courier).expect("placed").pos = world.map.secret_shops[0].pos;
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
    let fits = |item| purchase_fits(view, player, session.statics(), item);
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
        session.statics(),
        ItemId::HEALING_SALVE
    ));
    assert!(!purchase_fits(
        view,
        player,
        session.statics(),
        ItemId::CLARITY
    ));
}

#[test]
fn announced_home_shop_geometry_matches_server_execution_for_every_map_and_carrier() {
    use bota_proto::{MapId, Team};
    for map_id in [MapId(0), MapId(1), MapId(2)] {
        for team in [Team::Radiant, Team::Dire] {
            let mut config = super::support::config();
            config.map = map_id;
            config.picks[0].team = team;
            let map = bota_core::map_of(map_id);
            config.objectives = map.default_objectives;
            config.runes = map.default_runes;
            let info = config.info();
            assert_eq!(info.fountains, map.fountains);
            assert_eq!(info.shop_range, bota_core::rules::SHOP_RANGE);
            let statics = bota_client_core::MatchStatic::new(&info).expect("announced map");
            let mut world = World::for_match(&config, config.rng());
            let carriers = [
                world.seats[0].unit.unwrap(),
                world.seats[0].courier.unwrap(),
            ];
            let home = bota_core::fountain_pos(map, team);
            for (index, entity) in carriers.into_iter().enumerate() {
                for distance in [
                    0,
                    bota_core::rules::SHOP_RANGE,
                    bota_core::rules::SHOP_RANGE + 1,
                ] {
                    let direction = if team == Team::Radiant { 1 } else { -1 };
                    world.transform.get_mut(entity).unwrap().pos =
                        home + Vec2::from_ints(direction * distance, 0);
                    let view = world.view_full();
                    let context =
                        bota_client_core::purchase_context(&view, &view.players[0], &statics);
                    let carrier = if index == 0 {
                        context.hero
                    } else {
                        context.courier
                    }
                    .unwrap();
                    assert_eq!(
                        carrier.shops.base,
                        world.at_shop(entity),
                        "map={map_id:?}, team={team:?}, carrier={index}, distance={distance}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_dead_hero_at_home_buys_base_items_into_its_kit_through_orders() {
    let (mut local, mut session) = arranged(&STATES[0]);
    let world = local.world_mut();
    let hero = world.seats[0].unit.unwrap();
    world.kill_by_hp_removal(world.seats[1].unit.unwrap(), hero);
    let dead = local.step(&[&[], &[]]).expect("death tick");
    for message in dead.into_iter().next().unwrap() {
        session.ingest(message, &mut ()).expect("dead snapshot");
    }
    assert!(local.world().seats[0].dead(), "the hero is actually dead");
    let order = session
        .issue(bota_client_core::IssuedOrder::hero(Order::Buy {
            item: ItemId::IRON_BRANCH,
        }))
        .expect("a dead hero may issue a buy");
    let bought = local.step(&[&[order], &[]]).expect("purchase tick");
    for message in bought.into_iter().next().unwrap() {
        session.ingest(message, &mut ()).expect("purchase snapshot");
    }
    assert!(
        session
            .own_player()
            .unwrap()
            .kit
            .as_ref()
            .unwrap()
            .items
            .iter()
            .flatten()
            .any(|item| item.id == ItemId::IRON_BRANCH),
        "normal death purchases enter the persistent inventory"
    );
    assert!(
        session
            .own_player()
            .unwrap()
            .stash
            .as_ref()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn f07_review_empty_split_recipe_is_refused_without_an_item_bought_event() {
    use bota_proto::OutgoingOrder;
    use bota_proto::{EventKind, ServerMsg};
    let (mut local, _) = seated();
    let world = local.world_mut();
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).unwrap().pos = Vec2::from_ints(9000, 9000);
    world.inventory.get_mut(hero).unwrap().slots[0] = Some(fresh_stack(ItemId::BOOTS, 1));
    world.seats[0].stash.slots[0] = Some(fresh_stack(ItemId::GLOVES, 1));
    world.seats[0].stash.slots[1] = Some(fresh_stack(ItemId::BELT, 1));
    world.seats[0].gold = 10000;
    let gold = world.seats[0].gold;
    let stock: Vec<_> = world.shop_stock.iter().map(|s| s.left).collect();
    let order = OutgoingOrder {
        seq: 42,
        unit: None,
        order: Order::Buy {
            item: ItemId::POWER_TREADS,
        },
        seen: local.tick(),
        queue: false,
    };
    let stream = local.step(&[&[order], &[]]).expect("real order step");
    assert!(!stream[0].iter().any(|m|matches!(m,ServerMsg::Events {events,..} if events.iter().any(|e|matches!(e,EventKind::ItemBought {item: ItemId::POWER_TREADS,..})))), "an empty plan must emit no purchase event");
    assert!(
        stream[0].iter().any(|m| matches!(
            m,
            ServerMsg::OrderRejected {
                seq: 42,
                reason: bota_proto::RejectReason::NothingToBuy
            }
        )),
        "the empty purchase is specifically refused"
    );
    assert_eq!(local.world().seats[0].gold, gold);
    assert_eq!(
        local
            .world()
            .shop_stock
            .iter()
            .map(|s| s.left)
            .collect::<Vec<_>>(),
        stock
    );
    let mut events = Vec::new();
    assert!(
        !local
            .world_mut()
            .buy(BUYER, ItemId::POWER_TREADS, &mut events),
        "execution independently rejects an empty plan"
    );
    assert!(events.is_empty());
}
