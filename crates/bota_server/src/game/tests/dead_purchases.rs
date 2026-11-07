//! Purchases while awaiting respawn fill the persistent inventory, not an assumed physical shop.

use bota_client_core::{ClientSession, purchase_fits};
use bota_core::item_def;
use bota_proto::{ItemId, ItemSlot, Order, OutgoingOrder, RejectReason, ServerMsg, SlotId, Vec2};

use super::{client::seated, support::fresh_stack};
use crate::local::LocalMatch;

fn fallen() -> (LocalMatch, ClientSession) {
    let (mut local, session) = seated();
    local.step(&[&[], &[]]).expect("time before death");
    let world = local.world_mut();
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("placed").pos = Vec2::from_ints(9000, 9000);
    world.bury(vec![(hero, None)], &mut Vec::new());
    world.seats[0].respawn_left = 1000;
    world.seats[0].gold = 10_000;
    (local, session)
}

fn step(
    local: &mut LocalMatch,
    session: &mut ClientSession,
    order: Option<Order>,
) -> Vec<Vec<ServerMsg>> {
    let request: Vec<_> = order
        .into_iter()
        .map(|order| OutgoingOrder {
            seq: local.tick() + 1,
            unit: None,
            order,
            seen: local.tick(),
            queue: false,
        })
        .collect();
    let messages = local.step(&[&request, &[]]).expect("real server path");
    for message in &messages[0] {
        session
            .ingest(message.clone(), &mut ())
            .expect("valid client view");
    }
    messages
}

fn kit_has(session: &ClientSession, item: ItemId) -> bool {
    session
        .current()
        .expect("view")
        .player(SlotId(0))
        .expect("player")
        .kit
        .as_ref()
        .expect("dead kit")
        .items
        .iter()
        .flatten()
        .any(|held| held.id == item)
}

fn refused(messages: &[ServerMsg]) -> Option<RejectReason> {
    messages.iter().find_map(|message| {
        if let ServerMsg::OrderRejected { reason, .. } = message {
            Some(*reason)
        } else {
            None
        }
    })
}

#[test]
fn a_dead_purchase_enters_the_projected_kit_and_survives_respawn() {
    let (mut local, mut session) = fallen();
    let messages = step(
        &mut local,
        &mut session,
        Some(Order::Buy {
            item: ItemId::BOOTS,
        }),
    );
    assert_eq!(refused(&messages[0]), None);
    assert!(
        kit_has(&session, ItemId::BOOTS),
        "a dead buyer receives the purchase in its actual carried inventory"
    );
    assert!(
        session
            .current()
            .expect("view")
            .player(SlotId(0))
            .expect("player")
            .stash
            .as_ref()
            .expect("stash")
            .iter()
            .all(Option::is_none)
    );
    let hero = local.world().seats[0].unit.expect("hero handle persists");
    local.world_mut().seats[0].respawn_left = 1;
    step(&mut local, &mut session, None);
    assert!(!local.world().seats[0].dead());
    assert!(
        local
            .world()
            .inventory
            .get(hero)
            .expect("respawn inventory")
            .held()
            .any(|stack| stack.id == ItemId::BOOTS)
    );
}

#[test]
fn a_full_stash_does_not_block_a_dead_heros_available_inventory() {
    let (mut local, mut session) = fallen();
    local.world_mut().seats[0]
        .stash
        .slots
        .fill(Some(fresh_stack(ItemId::QUELLING_BLADE, 1)));
    step(&mut local, &mut session, None);
    let view = session.current().expect("view");
    let buyer = view.player(SlotId(0)).expect("buyer");
    assert!(
        purchase_fits(view, buyer, session.statics(), ItemId::BOOTS),
        "prediction counts the dead kit's free berth"
    );
    let messages = step(
        &mut local,
        &mut session,
        Some(Order::Buy {
            item: ItemId::BOOTS,
        }),
    );
    assert_eq!(refused(&messages[0]), None);
    assert!(kit_has(&session, ItemId::BOOTS));
    assert!(
        local.world().seats[0]
            .stash
            .slots
            .iter()
            .flatten()
            .all(|stack| stack.id == ItemId::QUELLING_BLADE)
    );
}

#[test]
fn a_dead_heros_full_inventory_uses_the_stash_then_refuses_without_space() {
    for full_stash in [false, true] {
        let (mut local, mut session) = fallen();
        let hero = local.world().seats[0].unit.expect("hero");
        local
            .world_mut()
            .inventory
            .get_mut(hero)
            .expect("bag")
            .slots
            .fill(Some(fresh_stack(ItemId::QUELLING_BLADE, 1)));
        if full_stash {
            local.world_mut().seats[0]
                .stash
                .slots
                .fill(Some(fresh_stack(ItemId::QUELLING_BLADE, 1)));
        }
        let gold = local.world().seats[0].gold;
        let messages = step(
            &mut local,
            &mut session,
            Some(Order::Buy {
                item: ItemId::BOOTS,
            }),
        );
        assert_eq!(
            refused(&messages[0]),
            full_stash.then_some(RejectReason::InventoryFull)
        );
        assert!(!kit_has(&session, ItemId::BOOTS));
        if full_stash {
            assert_eq!(local.world().seats[0].gold, gold);
        } else {
            assert!(
                local.world().seats[0]
                    .stash
                    .slots
                    .iter()
                    .flatten()
                    .any(|stack| stack.id == ItemId::BOOTS)
            );
        }
    }
}

#[test]
fn a_dead_purchase_merges_into_the_inventory_without_making_an_old_stack_sellable() {
    let (mut local, mut session) = fallen();
    let hero = local.world().seats[0].unit.expect("hero");
    let mut old = fresh_stack(ItemId::CLARITY, 1);
    old.touched = true;
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots
        .fill(Some(old));
    local.world_mut().seats[0]
        .stash
        .slots
        .fill(Some(fresh_stack(ItemId::QUELLING_BLADE, 1)));
    let messages = step(
        &mut local,
        &mut session,
        Some(Order::Buy {
            item: ItemId::CLARITY,
        }),
    );
    assert_eq!(
        refused(&messages[0]),
        None,
        "an existing stack can receive the charge even with full containers"
    );
    assert_eq!(
        local.world().inventory.get(hero).expect("bag").slots[0]
            .expect("stack")
            .charges,
        2
    );
    let gold = local.world().seats[0].gold;
    let messages = step(
        &mut local,
        &mut session,
        Some(Order::Sell { slot: ItemSlot(0) }),
    );
    assert_eq!(
        refused(&messages[0]),
        Some(RejectReason::HeroDead),
        "buying one charge cannot expose the pre-death stack to sale"
    );
    assert_eq!(local.world().seats[0].gold, gold);
}

#[test]
fn the_dead_heros_kit_does_not_grant_secret_shop_access() {
    let (mut local, mut session) = fallen();
    let messages = step(
        &mut local,
        &mut session,
        Some(Order::Buy {
            item: ItemId::DEMON_EDGE,
        }),
    );
    assert_eq!(refused(&messages[0]), Some(RejectReason::NotAtShop));
    assert!(!kit_has(&session, ItemId::DEMON_EDGE));
    let courier = local.world().seats[0].courier.expect("courier");
    let shop = local.world().map.secret_shops[0].pos;
    local
        .world_mut()
        .transform
        .get_mut(courier)
        .expect("courier position")
        .pos = shop;
    let messages = step(
        &mut local,
        &mut session,
        Some(Order::Buy {
            item: ItemId::DEMON_EDGE,
        }),
    );
    assert_eq!(refused(&messages[0]), None);
    assert!(
        local
            .world()
            .inventory
            .get(courier)
            .expect("courier bag")
            .held()
            .any(|stack| stack.id == ItemId::DEMON_EDGE)
    );
    assert!(
        !kit_has(&session, ItemId::DEMON_EDGE),
        "secret purchases still use the physical courier"
    );
}

#[test]
fn an_inventory_purchase_made_during_this_death_can_be_refunded() {
    let (mut local, mut session) = fallen();
    let before = local.world().seats[0].gold;
    step(
        &mut local,
        &mut session,
        Some(Order::Buy {
            item: ItemId::BOOTS,
        }),
    );
    assert!(kit_has(&session, ItemId::BOOTS));
    let messages = step(
        &mut local,
        &mut session,
        Some(Order::Sell { slot: ItemSlot(0) }),
    );
    assert_eq!(refused(&messages[0]), None);
    assert!(!kit_has(&session, ItemId::BOOTS));
    assert_eq!(local.world().seats[0].gold, before);
    assert!(item_def(ItemId::BOOTS).expect("boots").cost > 0);
}

#[test]
fn a_paid_scroll_bought_during_death_still_requires_a_shop_to_sell() {
    for at_shop in [false, true] {
        let (mut local, mut session) = fallen();
        let hero = local.world().seats[0].unit.expect("hero");
        local
            .world_mut()
            .inventory
            .get_mut(hero)
            .expect("bag")
            .town_portal = Some(None);
        step(
            &mut local,
            &mut session,
            Some(Order::Buy {
                item: ItemId::TOWN_PORTAL_SCROLL,
            }),
        );
        let kit = session
            .current()
            .expect("view")
            .player(SlotId(0))
            .expect("player")
            .kit
            .as_ref()
            .expect("dead kit");
        assert_eq!(
            kit.sellable_slots & (1u32 << bota_client_core::TOWN_PORTAL_SLOT),
            0
        );
        if at_shop {
            local.world_mut().seats[0].respawn_left = 1;
            step(&mut local, &mut session, None);
            assert!(
                !local.world().seats[0].dead(),
                "positive control respawned at the shop"
            );
        }
        let before = local.world().seats[0].gold;
        let messages = step(
            &mut local,
            &mut session,
            Some(Order::Sell {
                slot: ItemSlot(bota_client_core::TOWN_PORTAL_SLOT),
            }),
        );
        assert_eq!(
            refused(&messages[0]),
            (!at_shop).then_some(RejectReason::NotAtShop)
        );
        assert_eq!(local.world().seats[0].gold > before, at_shop);
    }
}

#[test]
fn dead_sale_eligibility_is_private_and_excludes_predeath_items() {
    let (mut local, mut session) = fallen();
    let hero = local.world().seats[0].unit.expect("hero");
    local
        .world_mut()
        .inventory
        .get_mut(hero)
        .expect("bag")
        .slots[1] = Some(fresh_stack(ItemId::IRON_BRANCH, 1));
    step(
        &mut local,
        &mut session,
        Some(Order::Buy {
            item: ItemId::BOOTS,
        }),
    );
    let kit = session
        .current()
        .expect("view")
        .player(SlotId(0))
        .expect("player")
        .kit
        .as_ref()
        .expect("own kit");
    assert_ne!(kit.sellable_slots & 1, 0, "new boots may be refunded");
    assert!(
        !kit.reincarnating,
        "a normal death supplies a receiving kit"
    );
    assert_eq!(kit.sellable_slots & 2, 0, "old branch may not be sold");
    assert_eq!(
        local
            .world()
            .view_full()
            .player(SlotId(0))
            .expect("full scoreboard")
            .kit
            .as_ref()
            .expect("full kit")
            .sellable_slots,
        kit.sellable_slots
    );
    assert!(
        local
            .world()
            .view(bota_proto::Team::Dire)
            .player(SlotId(0))
            .expect("enemy scoreboard")
            .kit
            .is_none()
    );
}

#[test]
fn a_respawning_hero_can_retrieve_its_stash_away_from_either_shop() {
    let (mut local, mut session) = fallen();
    local.world_mut().seats[0].stash.slots[0] = Some(fresh_stack(ItemId::QUELLING_BLADE, 1));
    let messages = step(
        &mut local,
        &mut session,
        Some(Order::Swap {
            from: ItemSlot(bota_client_core::BAG_SLOTS),
            to: ItemSlot(0),
        }),
    );
    assert_eq!(refused(&messages[0]), None);
    assert!(kit_has(&session, ItemId::QUELLING_BLADE));
    assert!(local.world().seats[0].stash.slots[0].is_none());
}
