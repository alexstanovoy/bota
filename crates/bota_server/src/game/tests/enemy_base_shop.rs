//! Either base shop serves the visitor's own inventory and stash.

use bota_core::{hero_spawn_pos, item_def};
use bota_proto::{ItemId, ItemSlot, Order, OutgoingOrder, RejectReason, ServerMsg, SlotId, Team};

use super::{client::seated, support::fresh_stack};
use crate::game::BAG_SLOTS;
use crate::local::LocalMatch;

fn visit_enemy_base(buyer: usize) -> LocalMatch {
    let (mut local, _) = seated();
    let world = local.world_mut();
    let hero = world.seats[buyer].unit.expect("hero");
    let enemy = if world.seats[buyer].team == Team::Radiant {
        Team::Dire
    } else {
        Team::Radiant
    };
    world.transform.get_mut(hero).expect("placed").pos = hero_spawn_pos(world.map, enemy);
    world.seats[buyer].gold = 10_000;
    local
}

fn order(local: &mut LocalMatch, buyer: usize, action: Order) -> Vec<ServerMsg> {
    let issued = OutgoingOrder {
        seq: local.tick() + 1,
        unit: None,
        order: action,
        seen: local.tick(),
        queue: false,
    };
    let mut requests = vec![Vec::new(); local.world().seats.len()];
    requests[buyer].push(issued);
    let requests: Vec<_> = requests.iter().map(Vec::as_slice).collect();
    local
        .step(&requests)
        .expect("real server order step")
        .swap_remove(buyer)
}

fn accepted(messages: &[ServerMsg]) {
    let rejected: Vec<_> = messages
        .iter()
        .filter_map(|message| {
            if let ServerMsg::OrderRejected { reason, .. } = message {
                Some(reason)
            } else {
                None
            }
        })
        .collect();
    assert!(
        rejected.is_empty(),
        "the shop order must be accepted: {rejected:?}"
    );
}

#[test]
fn either_hero_buys_into_its_inventory_at_the_enemy_base() {
    for buyer in 0..2 {
        let mut local = visit_enemy_base(buyer);
        let hero = local.world().seats[buyer].unit.expect("hero");
        let messages = order(
            &mut local,
            buyer,
            Order::Buy {
                item: ItemId::BOOTS,
            },
        );
        accepted(&messages);
        assert_eq!(
            local.world().inventory.get(hero).expect("bag").slots[0].map(|item| item.id),
            Some(ItemId::BOOTS),
            "the visitor receives the purchase, rather than its remote stash"
        );
        assert!(
            local.world().seats[buyer]
                .stash
                .slots
                .iter()
                .all(Option::is_none)
        );
    }
}

#[test]
fn either_base_shop_reaches_the_same_visitors_stash() {
    for buyer in 0..2 {
        let mut local = visit_enemy_base(buyer);
        let hero = local.world().seats[buyer].unit.expect("hero");
        let other = 1 - buyer;
        let mut boots = fresh_stack(ItemId::BOOTS, 1);
        boots.owner = SlotId(buyer as u8);
        let mut branch = fresh_stack(ItemId::IRON_BRANCH, 1);
        branch.owner = SlotId(other as u8);
        local.world_mut().seats[buyer].stash.slots[0] = Some(boots);
        local.world_mut().seats[other].stash.slots[0] = Some(branch);
        accepted(&order(
            &mut local,
            buyer,
            Order::Swap {
                from: ItemSlot(BAG_SLOTS as u8),
                to: ItemSlot(0),
            },
        ));
        assert_eq!(
            local.world().inventory.get(hero).expect("bag").slots[0].map(|item| item.id),
            Some(ItemId::BOOTS)
        );
        accepted(&order(
            &mut local,
            buyer,
            Order::Swap {
                from: ItemSlot(0),
                to: ItemSlot(BAG_SLOTS as u8 + 1),
            },
        ));
        assert_eq!(
            local.world().seats[buyer].stash.slots[1].map(|item| item.id),
            Some(ItemId::BOOTS)
        );
        let own = local.world().seats[buyer].team;
        let home = hero_spawn_pos(local.world().map, own);
        local
            .world_mut()
            .transform
            .get_mut(hero)
            .expect("placed")
            .pos = home;
        accepted(&order(
            &mut local,
            buyer,
            Order::Swap {
                from: ItemSlot(BAG_SLOTS as u8 + 1),
                to: ItemSlot(0),
            },
        ));
        assert_eq!(
            local.world().inventory.get(hero).expect("bag").slots[0].map(|item| item.id),
            Some(ItemId::BOOTS)
        );
        assert_eq!(
            local.world().seats[other].stash.slots[0],
            Some(branch),
            "the opposing player's stash is never involved"
        );
    }
}

#[test]
fn an_enemy_base_sale_pays_the_visitor_immediately_instead_of_marking() {
    for buyer in 0..2 {
        let mut local = visit_enemy_base(buyer);
        let hero = local.world().seats[buyer].unit.expect("hero");
        let mut boots = fresh_stack(ItemId::BOOTS, 1);
        boots.owner = SlotId(buyer as u8);
        local
            .world_mut()
            .inventory
            .get_mut(hero)
            .expect("bag")
            .slots[0] = Some(boots);
        let gold = local.world().seats[buyer].gold;
        accepted(&order(&mut local, buyer, Order::Sell { slot: ItemSlot(0) }));
        assert!(
            local.world().inventory.get(hero).expect("bag").slots[0].is_none(),
            "selling at the enemy base must complete, not mark the item for a later trip home"
        );
        assert!(
            local.world().seats[buyer].gold >= gold + item_def(ItemId::BOOTS).expect("boots").cost,
            "the visitor receives the fresh purchase's refund"
        );
    }
}

#[test]
fn marked_items_settle_when_their_carrier_reaches_the_enemy_base() {
    let mut local = visit_enemy_base(0);
    let hero = local.world().seats[0].unit.expect("hero");
    let mut boots = fresh_stack(ItemId::BOOTS, 1);
    boots.for_sale = true;
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[0] = Some(boots);
    let gold = local.world().seats[0].gold;
    accepted(&order(
        &mut local,
        0,
        Order::Move {
            target: bota_proto::Target::None,
        },
    ));
    assert!(
        local.world().inventory.get(hero).expect("bag").slots[0].is_none(),
        "either base shop settles a marked item"
    );
    assert!(local.world().seats[0].gold >= gold + item_def(ItemId::BOOTS).expect("boots").cost);
}

#[test]
fn enemy_base_assembly_uses_the_visitors_inventory_and_stash() {
    let mut local = visit_enemy_base(0);
    let hero = local.world().seats[0].unit.expect("hero");
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[0] = Some(fresh_stack(ItemId::BOOTS, 1));
    local.world_mut().seats[0].stash.slots[0] = Some(fresh_stack(ItemId::GLOVES, 1));
    local.world_mut().seats[0].stash.slots[1] = Some(fresh_stack(ItemId::BAND, 1));
    accepted(&order(
        &mut local,
        0,
        Order::Move {
            target: bota_proto::Target::None,
        },
    ));
    assert!(
        local
            .world()
            .inventory
            .get(hero)
            .expect("bag")
            .slots
            .iter()
            .flatten()
            .any(|item| item.id == ItemId::POWER_TREADS)
    );
    assert!(
        local.world().seats[0]
            .stash
            .slots
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn stash_access_stops_outside_both_base_shops_and_at_secret_shops() {
    for secret in [false, true] {
        let mut local = visit_enemy_base(0);
        let hero = local.world().seats[0].unit.expect("hero");
        let at = if secret {
            local.world().map.secret_shops[0].pos
        } else {
            bota_proto::Vec2::from_ints(9000, 9000)
        };
        local
            .world_mut()
            .transform
            .get_mut(hero)
            .expect("placed")
            .pos = at;
        local.world_mut().seats[0].stash.slots[0] = Some(fresh_stack(ItemId::BOOTS, 1));
        let messages = order(
            &mut local,
            0,
            Order::Swap {
                from: ItemSlot(BAG_SLOTS as u8),
                to: ItemSlot(0),
            },
        );
        assert!(
            messages.iter().any(|message| matches!(
                message,
                ServerMsg::OrderRejected {
                    reason: RejectReason::NotAtShop,
                    ..
                }
            )),
            "stash access is base-shop-only: {messages:?}"
        );
        assert!(local.world().inventory.get(hero).expect("bag").slots[0].is_none());
        assert!(local.world().seats[0].stash.slots[0].is_some());
    }
}

#[test]
fn the_enemy_base_does_not_allow_selling_a_free_starting_scroll() {
    let mut local = visit_enemy_base(0);
    let messages = order(
        &mut local,
        0,
        Order::Sell {
            slot: ItemSlot::TOWN_PORTAL,
        },
    );
    assert!(
        messages.iter().any(|message| matches!(
            message,
            ServerMsg::OrderRejected {
                reason: RejectReason::Unsellable,
                ..
            }
        )),
        "the initial unsellable charge remains protected at either shop"
    );
}
