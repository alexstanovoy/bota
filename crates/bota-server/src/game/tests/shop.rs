//! Buying, selling, the stash and items on the ground.

use crate::game::World;
use crate::game::rules;

use super::support::*;

#[test]
fn buying_needs_the_shop() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let away = bota_proto::Vec2::from_ints(9600, 9216);
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        away,
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let mut seat = crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(0),
        rules::STARTING_GOLD,
        rules::STASH_SLOTS,
    );
    seat.unit = Some(hero);
    world.seats.push(seat);
    world.settle();
    let mut events = Vec::new();
    let salve = bota_proto::ItemId(crate::game::ITEM_HEALING_SALVE);
    assert!(
        world.buy(bota_proto::SlotId(0), salve, &mut events),
        "out in the lane it buys all the same"
    );
    assert_eq!(
        world.seats[0].stash.slots[0].map(|stack| stack.id),
        Some(salve),
        "and what it bought waits in the stash"
    );
    assert!(
        world.inventory.get(hero).expect("has a bag").held().count() == 0,
        "nothing reaches its hands out there"
    );
    if let Some(at) = world.transform.get_mut(hero) {
        at.pos = crate::game::fountain_pos(map, bota_proto::Team::Radiant);
    }
    assert!(
        world.buy(bota_proto::SlotId(0), salve, &mut events),
        "at its own shop it may"
    );
    assert!(world.seats[0].gold < rules::STARTING_GOLD, "and it paid");
}

#[test]
fn buying_through_a_command_returns_the_purchase_event() {
    let mut world = World::for_match(&config(), config().rng());
    let item = bota_proto::ItemId(crate::game::ITEM_HEALING_SALVE);

    let events = world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Buy { item },
    }]);

    assert!(events.iter().any(|event| {
        event.kind
            == bota_proto::EventKind::ItemBought {
                slot: bota_proto::SlotId(0),
                item,
            }
    }));
}

#[test]
fn what_a_hero_bought_is_in_the_view_its_side_is_sent() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    let mut events = Vec::new();
    assert!(
        world.buy(bota_proto::SlotId(0), boots, &mut events),
        "standing at its own shop, it may buy"
    );
    let view = world.view(bota_proto::Team::Radiant);
    let mine = view
        .units
        .iter()
        .find(|unit| unit.id == crate::game::wire_id(hero))
        .expect("its own hero is in its own view");
    assert_eq!(
        mine.items.first().and_then(|slot| slot.map(|item| item.id)),
        Some(boots),
        "and what it bought is in the bag it is sent"
    );
    assert_eq!(
        mine.items.len(),
        rules::INVENTORY_SLOTS + rules::BACKPACK_SLOTS,
        "every slot keeps its place, held or not"
    );
    let seat = view
        .players
        .iter()
        .find(|player| player.slot == bota_proto::SlotId(0))
        .expect("its own seat");
    assert_eq!(
        seat.stash.as_ref().map(|stash| stash.len()),
        Some(rules::STASH_SLOTS),
        "and the stash is sent with its slots"
    );
}

#[test]
fn what_waits_in_the_stash_can_be_taken_into_the_bag() {
    let (mut world, hero, boots) = a_hero_at_the_shop();
    assert!(
        world.move_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS, 0),
        "at its own shop the stash takes part"
    );
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|stack| stack.id),
        Some(boots),
        "and the item is in hand"
    );
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "with nothing left behind it"
    );
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.move_speed),
        Some(bota_proto::Fixed::from_int(
            crate::game::HERO.move_speed + 45
        )),
        "and it works at once, being no backpack it came from"
    );
}

#[test]
fn the_backpack_takes_from_the_stash_too() {
    let (mut world, hero, boots) = a_hero_at_the_shop();
    let pocket = rules::INVENTORY_SLOTS;
    assert!(
        world.move_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS, pocket),
        "the pocket is a place like any other"
    );
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[pocket].map(|stack| stack.id),
        Some(boots),
        "and holds it"
    );
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.move_speed),
        Some(bota_proto::Fixed::from_int(crate::game::HERO.move_speed)),
        "carried inert, it adds nothing"
    );
    // Out of the pocket into the inventory, it waits before it works.
    assert!(world.move_item(bota_proto::SlotId(0), hero, pocket, 0));
    world.step();
    assert_eq!(
        world.stats.get(hero).map(|s| s.move_speed),
        Some(bota_proto::Fixed::from_int(crate::game::HERO.move_speed)),
        "just out of the pocket it is still inert"
    );
    for _ in 0..rules::BACKPACK_MUTE_TICKS {
        world.step();
    }
    assert_eq!(
        world.stats.get(hero).map(|s| s.move_speed),
        Some(bota_proto::Fixed::from_int(
            crate::game::HERO.move_speed + 45
        )),
        "and works once the wait is out"
    );
}

#[test]
fn a_backpack_swap_projects_mute_until_the_exact_boundary() {
    let (mut world, hero, boots) = a_hero_at_the_shop();
    let pocket = rules::INVENTORY_SLOTS;
    assert!(
        world.move_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS, pocket),
        "the stash item moves into the backpack"
    );
    world.inventory.get_mut(hero).expect("has a bag").slots[0] =
        Some(a_stack_of(crate::game::ITEM_IRON_BRANCH, 0));
    assert!(
        world.move_item(bota_proto::SlotId(0), hero, pocket, 0),
        "the backpack item swaps into the inventory"
    );

    let projected = |world: &World| {
        let view = world.view(bota_proto::Team::Radiant);
        view.units
            .iter()
            .find(|unit| unit.id == crate::game::wire_id(hero))
            .and_then(|unit| unit.items.first().copied().flatten())
            .expect("the item is visible to its seat")
    };
    let item = projected(&world);
    assert_eq!(item.id, boots, "the backpack item landed in front");
    assert!(item.mute_left > 0, "the projected mute starts positive");
    assert_eq!(
        item.mute_left,
        slot_of(&world, hero, 0).expect("held in front").mute,
        "the view carries the exact stack mute"
    );

    for _ in 1..rules::BACKPACK_MUTE_TICKS {
        world.step();
    }
    assert_eq!(projected(&world).mute_left, 1, "one tick remains");
    world.step();
    assert_eq!(projected(&world).mute_left, 0, "the boundary is ready");
}

#[test]
fn the_stash_is_out_of_reach_away_from_the_shop() {
    let (mut world, hero, _boots) = a_hero_at_the_shop();
    world.transform.get_mut(hero).expect("hero").pos = bota_proto::Vec2::from_ints(9600, 9216);
    assert!(
        !world.move_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS, 0),
        "out in the lane the stash cannot be reached into"
    );
}

#[test]
fn selling_soon_after_buying_pays_the_whole_price_back() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let purse = world.seats[0].gold;
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    let mut events = Vec::new();
    assert!(world.buy(bota_proto::SlotId(0), boots, &mut events));
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, 0),
        "and sells it back"
    );
    assert_eq!(world.seats[0].gold, purse, "nothing was lost on it");
}

#[test]
fn the_stash_sells_from_anywhere_and_a_bag_far_out_only_marks() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    let stack = crate::game::ItemStack {
        id: boots,
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: true,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    };
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(stack);
    }
    world.seats[0].stash.slots[0] = Some(stack);
    // Out in the lane, well away from its own shop.
    world.transform.get_mut(hero).expect("standing").pos =
        world.courier_home(bota_proto::Team::Radiant) + bota_proto::Vec2::from_ints(4000, 0);
    world.settle();
    let purse = world.seats[0].gold;
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, 0),
        "the ask is taken out there"
    );
    assert_eq!(world.seats[0].gold, purse, "but as a mark, not a sale");
    assert!(
        world
            .inventory
            .get(hero)
            .and_then(|bag| bag.slots[0])
            .is_some_and(|held| held.for_sale),
        "the stack stays in hand, marked"
    );
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS),
        "what waits in the stash is already at the shop"
    );
    assert!(world.seats[0].gold > purse, "and paid for");
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "and gone from the stash"
    );
    // Out in the lane the order is allowed now: it marks.
    let sell_bag = bota_proto::Order::Sell {
        slot: bota_proto::ItemSlot(0),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &sell_bag),
        Ok(())
    );
}

#[test]
fn buying_a_built_item_buys_only_the_parts_it_lacks() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let treads = bota_proto::ItemId(crate::game::ITEM_POWER_TREADS);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    let before = world.seats[0].gold;
    let mut events = Vec::new();
    assert!(
        world.buy(bota_proto::SlotId(0), treads, &mut events),
        "bought"
    );
    assert_eq!(
        before - world.seats[0].gold,
        price_of(crate::game::ITEM_POWER_TREADS) - price_of(crate::game::ITEM_BOOTS),
        "the boots already in hand are not paid for twice"
    );
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(treads),
        "and the parts build themselves into the whole"
    );
    assert!(
        world
            .inventory
            .get(hero)
            .expect("has a bag")
            .slots
            .iter()
            .skip(1)
            .all(|slot| slot.is_none()),
        "leaving nothing of what went into it"
    );
}

#[test]
fn composite_buy_order_pays_missing_cost_and_assembles_owned_parts() {
    use crate::game::{BAG_SLOTS, ITEM_CIRCLET, ITEM_WRAITH_BAND};
    use bota_proto::{EventKind, ItemId, ItemSlot, Order, RejectReason, SlotId};

    assert_eq!(price_of(ITEM_WRAITH_BAND), 505);
    assert_eq!(price_of(ITEM_CIRCLET), 155);
    for location in [0, rules::INVENTORY_SLOTS, BAG_SLOTS] {
        let (mut world, hero) = a_hero_with_gold(349);
        let circlet = a_stack_of(ITEM_CIRCLET, 0);
        if location == BAG_SLOTS {
            world.seats[0].stash.slots[0] = Some(circlet);
        } else {
            world.inventory.get_mut(hero).expect("bag").slots[location] = Some(circlet);
        }
        let item = ItemId(ITEM_WRAITH_BAND);
        reject_purchase_without_mutation(&mut world, hero, item, RejectReason::NotEnoughGold);
        world.seats[0].gold = 350;

        let events = advance_validated(&mut world, None, Order::Buy { item });

        assert_eq!(world.seats[0].gold, 0);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind
                    == EventKind::ItemBought {
                        slot: SlotId(0),
                        item
                    })
                .count(),
            1
        );
        if location == BAG_SLOTS {
            assert_eq!(world.seats[0].stash.slots[0], Some(circlet));
            assert_eq!(world.inventory.get(hero).expect("bag").held().count(), 2);
            advance_validated(
                &mut world,
                None,
                Order::Swap {
                    from: ItemSlot(BAG_SLOTS as u8),
                    to: ItemSlot(2),
                },
            );
        }
        let bag = world.inventory.get(hero).expect("bag");
        assert_eq!(bag.held().count(), 1);
        assert_eq!(bag.held().next().map(|stack| stack.id), Some(item));
        assert_eq!(world.seats[0].stash.held().count(), 0);
    }
}

#[test]
fn composite_buy_order_requires_room_for_every_missing_part_before_assembly() {
    use crate::game::{
        ITEM_BOOTS, ITEM_CIRCLET, ITEM_RECIPE_WRAITH_BAND, ITEM_SLIPPERS, ITEM_WRAITH_BAND,
    };
    use bota_proto::{ItemId, Order, RejectReason};

    for remote in [false, true] {
        let (mut world, hero) = a_hero_with_gold(505);
        if !remote {
            world
                .inventory
                .get_mut(hero)
                .expect("bag")
                .slots
                .fill(Some(a_stack_of(ITEM_BOOTS, 0)));
        }
        world.inventory.get_mut(hero).expect("bag").slots[0] = Some(a_stack_of(ITEM_CIRCLET, 0));
        world.seats[0]
            .stash
            .slots
            .fill(Some(a_stack_of(ITEM_BOOTS, 0)));
        if remote {
            world.transform.get_mut(hero).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
        }
        let item = ItemId(ITEM_WRAITH_BAND);
        world.seats[0].stash.slots[4] = None;
        reject_purchase_without_mutation(&mut world, hero, item, RejectReason::InventoryFull);
        world.seats[0].stash.slots[5] = None;
        let courier = the_courier(&world);

        advance_validated(&mut world, Some(courier), Order::Buy { item });

        assert_eq!(world.seats[0].gold, 155);
        assert_eq!(
            world.seats[0].stash.slots[4].map(|stack| stack.id),
            Some(ItemId(ITEM_SLIPPERS))
        );
        assert_eq!(
            world.seats[0].stash.slots[5].map(|stack| stack.id),
            Some(ItemId(ITEM_RECIPE_WRAITH_BAND))
        );
        assert_eq!(
            slot_of(&world, hero, 0).map(|stack| stack.id),
            Some(ItemId(ITEM_CIRCLET))
        );
    }
}

#[test]
fn composite_buy_order_does_not_credit_foreign_or_sale_marked_parts() {
    use crate::game::{ITEM_CIRCLET, ITEM_WRAITH_BAND};
    use bota_proto::{ItemId, RejectReason};

    for in_stash in [false, true] {
        for (owner, for_sale) in [(1, false), (0, true)] {
            let (mut world, hero) = a_hero_with_gold(350);
            let mut circlet = a_stack_of(ITEM_CIRCLET, owner);
            circlet.for_sale = for_sale;
            if in_stash {
                world.seats[0].stash.slots[0] = Some(circlet);
            } else {
                world.inventory.get_mut(hero).expect("bag").slots[0] = Some(circlet);
            }

            reject_purchase_without_mutation(
                &mut world,
                hero,
                ItemId(ITEM_WRAITH_BAND),
                RejectReason::NotEnoughGold,
            );
        }
    }
}

#[test]
fn an_item_laid_down_lies_where_it_was_aimed() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    let from = world.transform.get(hero).expect("stands").pos;
    let spot = bota_proto::Vec2 {
        x: from.x + rules::units(100),
        y: from.y,
    };
    assert!(world.put_item(hero, 0, bota_proto::Target::Pos(spot)));
    world.step();
    assert!(
        slot_of(&world, hero, 0).is_none(),
        "the bag slot gave it up"
    );
    let lying = on_the_ground(&world);
    assert_eq!(lying.len(), 1, "one item lies on the ground");
    assert_eq!(
        world.transform.get(lying[0]).map(|t| t.pos),
        Some(spot),
        "where it was aimed"
    );
}

#[test]
fn an_item_aimed_past_reach_walks_its_carrier_in_first() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    let from = world.transform.get(hero).expect("stands").pos;
    let spot = bota_proto::Vec2 {
        x: from.x + rules::units(1000),
        y: from.y + rules::units(1000),
    };
    assert!(world.put_item(hero, 0, bota_proto::Target::Pos(spot)));
    world.step();
    assert!(on_the_ground(&world).is_empty(), "too far to lay at once");
    for _ in 0..300 {
        world.step();
    }
    let lying = on_the_ground(&world);
    assert_eq!(lying.len(), 1, "walked over and laid it");
    assert_eq!(world.transform.get(lying[0]).map(|t| t.pos), Some(spot));
    let stood = world.transform.get(hero).expect("stands").pos;
    assert!(
        stood.within(spot, rules::units(rules::PUT_ITEM_RANGE)),
        "from within reach"
    );
}

#[test]
fn what_is_handed_over_lands_in_the_first_free_slot() {
    let (mut world, hero) = a_hero_with_gold(0);
    let courier = world.seats[0].courier.expect("stands with one");
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    assert!(world.put_item(
        hero,
        0,
        bota_proto::Target::Unit(crate::game::wire_id(courier))
    ));
    world.step();
    assert!(
        slot_of(&world, hero, 0).is_none(),
        "out of the hero's hands"
    );
    assert_eq!(
        world
            .inventory
            .get(courier)
            .and_then(|bag| bag.slots[0])
            .map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "and into the courier's"
    );
    // And back again the same way.
    assert!(world.put_item(
        courier,
        0,
        bota_proto::Target::Unit(crate::game::wire_id(hero))
    ));
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "handed back"
    );
    assert!(
        world
            .inventory
            .get(courier)
            .is_some_and(|bag| bag.held().count() == 0),
        "and the courier's hands are empty"
    );
}

#[test]
fn a_bag_with_no_room_is_handed_nothing() {
    let (mut world, hero) = a_hero_with_gold(0);
    let courier = world.seats[0].courier.expect("stands with one");
    if let Some(bag) = world.inventory.get_mut(courier) {
        for slot in bag.slots.iter_mut() {
            *slot = Some(a_stack_of(crate::game::ITEM_IRON_BRANCH, 0));
        }
    }
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    assert!(world.put_item(
        hero,
        0,
        bota_proto::Target::Unit(crate::game::wire_id(courier))
    ));
    world.step();
    assert!(slot_of(&world, hero, 0).is_some(), "kept where it was");
    assert!(world.handling.get(hero).is_none(), "and the errand is over");
}

#[test]
fn an_enemy_may_take_what_lies_on_the_ground_but_never_sell_it() {
    let (mut world, _hero) = a_hero_with_gold(0);
    let mid = bota_proto::Vec2::from_ints(8000, 8000);
    let enemy = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2 {
            x: mid.x + rules::units(100),
            y: mid.y,
        },
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(1),
        bota_proto::Team::Dire,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[1].unit = Some(enemy);
    world.settle();
    let lying = world.lay_loot(a_stack_of(crate::game::ITEM_BOOTS, 0), mid);
    assert!(world.take_item(enemy, crate::game::wire_id(lying)));
    world.step();
    assert_eq!(
        slot_of(&world, enemy, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "anybody with a bag may take it"
    );
    assert!(
        on_the_ground(&world).is_empty(),
        "and it is gone from the ground"
    );
    // At its own shop it is still not the enemy's to cash.
    if let Some(t) = world.transform.get_mut(enemy) {
        t.pos = crate::game::fountain_pos(world.map, bota_proto::Team::Dire);
    }
    let before = world.seats[1].gold;
    assert!(
        !world.sell_item(bota_proto::SlotId(1), enemy, 0),
        "not this seat's to sell"
    );
    assert!(slot_of(&world, enemy, 0).is_some(), "so it stays in hand");
    assert_eq!(world.seats[1].gold, before, "and pays nothing");
}

#[test]
fn what_was_muted_stays_muted_across_the_ground() {
    let (mut world, hero) = a_hero_with_gold(0);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            mute: 100,
            ..a_stack_of(crate::game::ITEM_BOOTS, 0)
        });
    }
    assert!(world.put_item(hero, 0, bota_proto::Target::None));
    world.step();
    let lying = on_the_ground(&world);
    assert_eq!(lying.len(), 1);
    assert!(world.take_item(hero, crate::game::wire_id(lying[0])));
    world.step();
    assert!(
        slot_of(&world, hero, 0).is_some_and(|stack| stack.mute > 90),
        "the ground is no way around the backpack mute"
    );
}

#[test]
fn selling_away_from_the_shop_marks_the_stack_and_a_second_ask_unmarks_it() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = bota_proto::Vec2::from_ints(8000, 8000);
    }
    let before = world.seats[0].gold;
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, 0),
        "away from the shop the ask is taken"
    );
    assert!(
        slot_of(&world, hero, 0).is_some_and(|stack| stack.for_sale),
        "as a mark rather than a sale"
    );
    world.step();
    assert!(
        slot_of(&world, hero, 0).is_some(),
        "and nothing sells this far out"
    );
    assert_eq!(world.seats[0].gold, before);
    assert!(
        world.sell_item(bota_proto::SlotId(0), hero, 0),
        "asked again"
    );
    assert!(
        slot_of(&world, hero, 0).is_some_and(|stack| !stack.for_sale),
        "the mark is off"
    );
}

#[test]
fn a_marked_stack_sells_the_moment_it_reaches_the_shop() {
    let (mut world, hero) = a_hero_with_gold(0);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 0));
    }
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = bota_proto::Vec2::from_ints(8000, 8000);
    }
    assert!(world.sell_item(bota_proto::SlotId(0), hero, 0), "marked");
    let before = world.seats[0].gold;
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = crate::game::fountain_pos(world.map, bota_proto::Team::Radiant);
    }
    world.step();
    assert!(slot_of(&world, hero, 0).is_none(), "sold on arrival");
    let half = price_of(crate::game::ITEM_BOOTS) * rules::SELL_PCT / 100;
    let gained = world.seats[0].gold - before;
    assert!(
        gained >= half && gained <= half + 1,
        "for its part of the price, {gained} against {half}"
    );
}

#[test]
fn a_courier_called_empty_still_collects_what_is_marked() {
    let (mut world, hero) = a_hero_with_gold(0);
    let courier = world.seats[0].courier.expect("stands with one");
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 0));
    }
    let shop = crate::game::fountain_pos(world.map, bota_proto::Team::Radiant);
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = shop + bota_proto::Vec2::from_ints(1500, 1500);
    }
    assert!(world.sell_item(bota_proto::SlotId(0), hero, 0), "marked");
    let before = world.seats[0].gold;
    let half = price_of(crate::game::ITEM_BOOTS) * rules::SELL_PCT / 100;
    assert!(world.courier_deliver(courier), "called with an empty bag");
    for _ in 0..1500 {
        world.step();
        if world.seats[0].gold >= before + half {
            break;
        }
    }
    assert!(
        slot_of(&world, hero, 0).is_none(),
        "the mark went with the bird"
    );
    assert!(
        world.seats[0].gold >= before + half,
        "and came back as gold"
    );
    assert!(
        world.seats[0].stash.held().count() == 0,
        "sold at the shop rather than shelved"
    );
}

#[test]
fn what_another_seat_bought_is_not_yours_to_sell() {
    let (mut world, hero) = a_hero_with_gold(0);
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 1));
    }
    let before = world.seats[0].gold;
    assert!(
        !world.sell_item(bota_proto::SlotId(0), hero, 0),
        "the other seat bought it"
    );
    assert!(slot_of(&world, hero, 0).is_some(), "so it stays");
    assert_eq!(world.seats[0].gold, before);
}

#[test]
fn a_marked_part_or_a_borrowed_part_builds_nothing() {
    let (mut world, hero) = a_hero_with_gold(0);
    // Out of the shop's reach, or the marked part would sell before the
    // build ever looked at it.
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = bota_proto::Vec2::from_ints(8000, 8000);
    }
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(crate::game::ItemStack {
            for_sale: true,
            ..a_stack_of(crate::game::ITEM_BOOTS, 0)
        });
        bag.slots[1] = Some(a_stack_of(crate::game::ITEM_GLOVES, 0));
        bag.slots[2] = Some(a_stack_of(crate::game::ITEM_BELT, 0));
    }
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "a part marked for sale does not vanish into a build"
    );
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 0));
        bag.slots[1] = Some(a_stack_of(crate::game::ITEM_GLOVES, 1));
    }
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_BOOTS)),
        "nor does a part somebody else bought"
    );
    if let Some(bag) = world.inventory.get_mut(hero) {
        bag.slots[1] = Some(a_stack_of(crate::game::ITEM_GLOVES, 0));
    }
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(bota_proto::ItemId(crate::game::ITEM_POWER_TREADS)),
        "whole and owned, the parts come together"
    );
}

#[test]
fn a_later_order_calls_an_item_errand_off() {
    let (mut world, hero) = a_hero_with_gold(0);
    hand_item(&mut world, hero, crate::game::ITEM_BOOTS, 0);
    let from = world.transform.get(hero).expect("stands").pos;
    let spot = bota_proto::Vec2 {
        x: from.x + rules::units(1000),
        y: from.y + rules::units(1000),
    };
    assert!(world.put_item(hero, 0, bota_proto::Target::Pos(spot)));
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    for _ in 0..200 {
        world.step();
    }
    assert!(on_the_ground(&world).is_empty(), "nothing was laid");
    assert!(
        slot_of(&world, hero, 0).is_some(),
        "and the item never left"
    );
}

#[test]
fn what_lies_on_the_ground_is_seen_through_the_fog() {
    let (mut world, hero) = a_hero_with_gold(0);
    let beside = world.transform.get(hero).expect("stands").pos;
    world.lay_loot(a_stack_of(crate::game::ITEM_BOOTS, 0), beside);
    world.step();
    let ours = world.view(bota_proto::Team::Radiant);
    assert_eq!(ours.loot.len(), 1, "lying in our own light");
    assert_eq!(
        ours.loot[0].item,
        bota_proto::ItemId(crate::game::ITEM_BOOTS)
    );
    assert_eq!(ours.loot[0].charges, None, "boots hold no charges");
    let theirs = world.view(bota_proto::Team::Dire);
    assert!(theirs.loot.is_empty(), "the far side has no eyes on it");
}

#[test]
fn buying_a_second_of_something_buys_a_second_of_it() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let branch = bota_proto::ItemId(crate::game::ITEM_IRON_BRANCH);
    let mut events = Vec::new();
    for held in 1..=3 {
        let before = world.seats[0].gold;
        assert!(
            world.buy(bota_proto::SlotId(0), branch, &mut events),
            "bought the branch"
        );
        assert_eq!(
            before - world.seats[0].gold,
            price_of(crate::game::ITEM_IRON_BRANCH),
            "and paid for it"
        );
        assert_eq!(
            world
                .inventory
                .get(hero)
                .expect("has a bag")
                .slots
                .iter()
                .flatten()
                .filter(|stack| stack.id == branch)
                .count(),
            held,
            "one more branch in hand than before"
        );
    }
}

#[test]
fn buying_a_built_item_a_second_time_buys_its_parts_again() {
    let (mut world, hero) = a_hero_with_gold(10_000);
    let treads = bota_proto::ItemId(crate::game::ITEM_POWER_TREADS);
    let mut events = Vec::new();
    assert!(
        world.buy(bota_proto::SlotId(0), treads, &mut events),
        "bought the first pair"
    );
    world.step();
    let before = world.seats[0].gold;
    assert!(
        world.buy(bota_proto::SlotId(0), treads, &mut events),
        "bought a second pair"
    );
    assert_eq!(
        before - world.seats[0].gold,
        price_of(crate::game::ITEM_POWER_TREADS),
        "the pair already worn is no part of the new one"
    );
    world.step();
    assert_eq!(
        world
            .inventory
            .get(hero)
            .expect("has a bag")
            .slots
            .iter()
            .flatten()
            .filter(|stack| stack.id == treads)
            .count(),
        2,
        "and both pairs are in hand"
    );
}
