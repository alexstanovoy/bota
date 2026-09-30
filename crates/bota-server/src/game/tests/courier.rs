//! Couriers and their errands.

use crate::game::rules;
use crate::game::{MELEE_CREEP, World};
use bota_proto::Fixed;

use super::support::*;

#[test]
fn a_courier_brought_down_comes_back_in_its_own_time() {
    use crate::game::{Errand, wire_id};
    use bota_proto::{AbilitySlot, Order, RejectReason, SlotId, Target};

    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    let deliver = Order::Cast {
        slot: AbilitySlot(3),
        target: Target::None,
    };
    advance_validated(&mut world, Some(courier), deliver);
    assert_eq!(world.errand.get(courier), Some(&Errand::ToOwner));
    let mut events = Vec::new();
    world.bury(vec![(courier, None)], &mut events);
    world.step();
    assert!(world.seats[0].courier.is_none(), "it is gone");
    assert!(world.seats[0].courier_left > 0, "and a wait has started");
    let burst = Order::Cast {
        slot: AbilitySlot(2),
        target: Target::None,
    };
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(courier)), &burst),
        Err(RejectReason::NotYourUnit)
    );
    for _ in 0..rules::COURIER_RESPAWN_TICKS {
        world.step();
    }
    let back = world.seats[0].courier.expect("it came back");
    assert_ne!(back, courier, "as a new body");
    assert_eq!(world.errand.get(back), Some(&Errand::None));
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(courier)), &burst),
        Err(RejectReason::NotYourUnit)
    );
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(back)), &burst),
        Ok(())
    );
    assert_eq!(
        world.transform.get(back).map(|at| at.pos),
        Some(world.courier_home(bota_proto::Team::Radiant)),
        "at its own fountain"
    );
}

#[test]
fn a_courier_fetches_the_stash_and_hands_it_to_its_owner() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    world.seats[0].stash.slots[0] = Some(crate::game::ItemStack {
        id: boots,
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    });
    // Standing at the fountain, it takes what waits there on the next tick.
    assert!(world.courier_take_stash(courier));
    world.step();
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "the stash is empty"
    );
    assert_eq!(
        world.inventory.get(courier).expect("carries").slots[0].map(|held| held.id),
        Some(boots),
        "and the courier holds it"
    );
    // Sent out to its owner, it walks there and hands it over.
    world.transform.get_mut(hero).expect("standing").pos =
        world.courier_home(bota_proto::Team::Radiant) + bota_proto::Vec2::from_ints(900, 0);
    assert!(world.courier_deliver(courier));
    for _ in 0..400 {
        world.step();
        if world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0)
        {
            break;
        }
    }
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|held| held.id),
        Some(boots),
        "what it carried is in its owner's hands"
    );
    assert_eq!(
        world
            .inventory
            .get(courier)
            .expect("carries")
            .held()
            .count(),
        0,
        "and the courier carries nothing now"
    );
}

#[test]
fn an_order_goes_to_the_unit_it_names_and_only_to_ones_this_seat_drives() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let to = world.courier_home(bota_proto::Team::Radiant) + bota_proto::Vec2::from_ints(600, 0);
    let walk = bota_proto::Order::Move {
        target: bota_proto::Target::Pos(to),
    };
    // Naming nobody is the hero.
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &walk),
        Ok(())
    );
    // Naming its own courier is allowed, and the order lands on the courier.
    let named = Some(crate::game::wire_id(courier));
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), named, &walk),
        Ok(()),
        "a seat drives its own courier"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: named,
        order: walk,
    }]);
    assert!(
        matches!(
            world.orders.get(courier).map(|orders| orders.current),
            Some(crate::game::UnitOrder::Move { pos }) if pos == to
        ),
        "the courier was told, not the hero"
    );
    assert!(
        !matches!(
            world.orders.get(hero).map(|orders| orders.current),
            Some(crate::game::UnitOrder::Move { .. })
        ),
        "and the hero was left alone"
    );
    // Anything else is nobody this seat drives.
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(6000, 6000),
    );
    world.settle();
    assert_eq!(
        world.validate_order(
            bota_proto::SlotId(0),
            Some(crate::game::wire_id(creep)),
            &walk
        ),
        Err(bota_proto::RejectReason::NotYourUnit),
        "a creep of its own side is still not its to drive"
    );
}

#[test]
fn a_courier_told_to_go_at_a_unit_follows_it() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    let home = world.courier_home(bota_proto::Team::Radiant);
    // Something of its own side standing a way off, that then walks further.
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        home + bota_proto::Vec2::from_ints(700, 0),
    );
    world.settle();
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: Some(crate::game::wire_id(courier)),
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(mark)),
        },
    }]);
    for _ in 0..120 {
        world.step();
    }
    let near = |world: &World| {
        let at = world.transform.get(courier).expect("standing").pos;
        let to = world.transform.get(mark).expect("standing").pos;
        crate::game::isqrt64(at.distance_squared(to))
            <= i64::from(bota_proto::Fixed::from_int(150).raw)
    };
    assert!(near(&world), "it went to it");
    // Moved on, it is followed rather than left behind.
    world.transform.get_mut(mark).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(700, 900);
    for _ in 0..200 {
        world.step();
    }
    assert!(near(&world), "and it keeps up when the mark moves");
}

#[test]
fn a_courier_at_the_fountain_reaches_the_stash_itself() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    world.seats[0].stash.slots[0] = Some(crate::game::ItemStack {
        id: boots,
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    });
    // The hero is out in the lane, so the stash is nothing to it.
    world.transform.get_mut(hero).expect("standing").pos =
        world.courier_home(bota_proto::Team::Radiant) + bota_proto::Vec2::from_ints(4000, 0);
    assert!(
        !world.move_item(bota_proto::SlotId(0), hero, crate::game::BAG_SLOTS, 0),
        "out in the lane a hero cannot reach into the stash"
    );
    // The courier is standing at the fountain, so for it the stash is right
    // there.
    assert!(
        world.move_item(bota_proto::SlotId(0), courier, crate::game::BAG_SLOTS, 0),
        "the courier at the fountain reaches it"
    );
    assert_eq!(
        world.inventory.get(courier).expect("carries").slots[0].map(|held| held.id),
        Some(boots),
        "and what waited there is in its hands"
    );
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "the stash is empty"
    );
    // And back again.
    assert!(world.move_item(bota_proto::SlotId(0), courier, 0, crate::game::BAG_SLOTS));
    assert_eq!(
        world.seats[0].stash.slots[0].map(|held| held.id),
        Some(boots),
        "it goes back the same way"
    );
}

#[test]
fn courier_auxiliary_casts_keep_delivery_tracking_a_moving_owner() {
    use crate::game::{Errand, UnitOrder, ability, wire_id};
    use bota_proto::{AbilitySlot, EventKind, Order, RejectReason, SlotId, Target};

    for (slot, ability, cooldown) in [
        (2, ability::BURST, rules::COURIER_BURST_COOLDOWN),
        (4, ability::SHIELD, rules::COURIER_SHIELD_COOLDOWN),
    ] {
        let (mut world, hero, courier) = courier_on_delivery();
        let cargo = slot_of(&world, courier, 0);
        let speed = world.stats.get(courier).expect("settled").move_speed;
        let mana = world.mana.get(courier).map(|pool| pool.mana);
        let order = Order::Cast {
            slot: AbilitySlot(slot),
            target: Target::None,
        };

        let events = advance_validated(&mut world, Some(courier), order);

        assert_eq!(world.errand.get(courier), Some(&Errand::ToOwner));
        assert!(events.iter().any(|event| event.kind
            == EventKind::AbilityCast {
                caster: wire_id(courier),
                ability,
            }));
        assert_eq!(
            world.abilities.get(courier).expect("book").slots[usize::from(slot)].cooldown,
            cooldown
        );
        assert_eq!(world.mana.get(courier).map(|pool| pool.mana), mana);
        assert_eq!(
            world.validate_order(SlotId(0), Some(wire_id(courier)), &order),
            Err(RejectReason::OnCooldown)
        );
        world.advance(&[]);
        let stats = world.stats.get(courier).expect("settled");
        if ability == ability::BURST {
            assert_eq!(
                stats.move_speed,
                speed * Fixed::from_ratio(100 + rules::COURIER_BURST_PCT, 100)
            );
        } else {
            assert!(stats.invulnerable);
        }
        let moved = rules::DEMO_LANE_CORNERS[1];
        assert_ne!(world.transform.get(hero).expect("standing").pos, moved);
        world.transform.get_mut(hero).expect("standing").pos = moved;
        world.advance(&[]);
        assert_eq!(
            world.orders.get(courier).expect("orders").current,
            UnitOrder::Move { pos: moved }
        );
        for _ in 0..400 {
            world.advance(&[]);
            if slot_of(&world, hero, 0).is_some() {
                break;
            }
        }
        assert_eq!(slot_of(&world, hero, 0), cargo);
        assert_eq!(world.inventory.get(courier).expect("bag").held().count(), 0);
    }
}

#[test]
fn courier_return_move_and_stop_replace_delivery() {
    use crate::game::Errand;
    use bota_proto::{AbilitySlot, Order, Target, Vec2};

    for order in [
        Order::Cast {
            slot: AbilitySlot(1),
            target: Target::None,
        },
        Order::Move {
            target: Target::Pos(Vec2::from_ints(5000, 5000)),
        },
        Order::Move {
            target: Target::None,
        },
    ] {
        let (mut world, hero, courier) = courier_on_delivery();
        let returning = matches!(order, Order::Cast { .. });

        advance_validated(&mut world, Some(courier), order);

        assert_eq!(
            world.errand.get(courier),
            Some(&if returning {
                Errand::PutBack
            } else {
                Errand::None
            })
        );
        for _ in 0..30 {
            world.advance(&[]);
        }
        assert_eq!(world.inventory.get(hero).expect("bag").held().count(), 0);
        if returning {
            assert_eq!(world.seats[0].stash.held().count(), 1);
            assert_eq!(world.inventory.get(courier).expect("bag").held().count(), 0);
        } else {
            assert_eq!(world.errand.get(courier), Some(&Errand::None));
            assert_eq!(world.inventory.get(courier).expect("bag").held().count(), 1);
        }
    }
}

#[test]
fn a_courier_that_has_handed_over_turns_for_home() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let home = world.courier_home(bota_proto::Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(900, 0);
    world.seats[0].stash.slots[0] = Some(crate::game::ItemStack {
        id: bota_proto::ItemId(crate::game::ITEM_BOOTS),
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    });
    assert!(world.courier_take_stash(courier));
    world.step();
    assert!(world.courier_deliver(courier));
    for _ in 0..300 {
        world.step();
        if world.errand.get(courier) == Some(&crate::game::Errand::None) {
            break;
        }
    }
    assert!(
        world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0),
        "it handed over"
    );
    assert!(
        matches!(
            world.orders.get(courier).map(|orders| orders.current),
            Some(crate::game::UnitOrder::Move { pos }) if pos == home
        ),
        "and turned for home on its own"
    );
}

#[test]
fn taking_the_stash_carries_it_on_without_being_asked_twice() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let home = world.courier_home(bota_proto::Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(900, 0);
    world.seats[0].stash.slots[0] = Some(crate::game::ItemStack {
        id: bota_proto::ItemId(crate::game::ITEM_BOOTS),
        charges: 0,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    });
    assert!(world.courier_take_stash(courier));
    for _ in 0..300 {
        world.step();
        if world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0)
        {
            break;
        }
    }
    assert!(
        world.inventory.get(hero).expect("has a bag").held().count() > 0,
        "one press fetched it and brought it"
    );
}

#[test]
fn an_errand_with_nothing_to_do_sends_the_courier_home() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    let home = world.courier_home(bota_proto::Team::Radiant);
    world.transform.get_mut(courier).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(1500, 0);
    // Nothing in the stash and nothing in its hands.
    assert!(world.courier_take_stash(courier));
    world.step();
    assert_eq!(
        world.errand.get(courier),
        Some(&crate::game::Errand::GoingHome),
        "with nothing to take it goes home"
    );
    for _ in 0..400 {
        world.step();
    }
    assert_eq!(
        world.transform.get(courier).map(|at| at.pos),
        Some(home),
        "and gets there"
    );
}

#[test]
fn a_courier_whose_owner_fell_puts_what_it_carries_back() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    if let Some(bag) = world.inventory.get_mut(courier) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: bota_proto::ItemId(crate::game::ITEM_BOOTS),
            charges: 0,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    let mut events = Vec::new();
    world.bury(vec![(hero, None)], &mut events);
    assert!(world.courier_deliver(courier));
    for _ in 0..300 {
        world.step();
        if world.seats[0].stash.held().count() > 0 {
            break;
        }
    }
    assert!(
        world.seats[0].stash.held().count() > 0,
        "with nobody to hand to, it put it back in the stash"
    );
}

#[test]
fn a_shielded_courier_takes_nothing() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    world.step();
    let full = world.health.get(courier).expect("standing").hp;
    assert!(world.courier_shield(courier));
    world.step();
    world.push_hit(None, courier, 100, bota_proto::DamageKind::Pure);
    world.step();
    assert_eq!(
        world.health.get(courier).map(|health| health.hp),
        Some(full),
        "nothing gets through while it holds"
    );
    for _ in 0..rules::COURIER_SHIELD_TICKS {
        world.step();
    }
    world.push_hit(None, courier, 100, bota_proto::DamageKind::Pure);
    world.step();
    assert!(
        world.health.get(courier).expect("standing").hp < full,
        "and once it lifts the courier is a courier again"
    );
}

#[test]
fn a_courier_keeps_its_load_through_death() {
    let (mut world, _hero) = a_hero_with_gold(0);
    let courier = world.seats[0].courier.expect("stands with one");
    if let Some(bag) = world.inventory.get_mut(courier) {
        bag.slots[0] = Some(a_stack_of(crate::game::ITEM_BOOTS, 0));
    }
    let mut events = Vec::new();
    world.bury(vec![(courier, None)], &mut events);
    assert!(
        world.seats[0]
            .courier_kept
            .as_ref()
            .is_some_and(|bag| bag.held().count() == 1),
        "the load waits on the seat"
    );
    for _ in 0..rules::COURIER_RESPAWN_TICKS + 2 {
        world.step();
    }
    let back = world.seats[0].courier.expect("stands again");
    assert_ne!(back, courier, "in a new body");
    assert_eq!(
        world.inventory.get(back).map(|bag| bag.held().count()),
        Some(1),
        "with the load back aboard"
    );
}

#[test]
fn a_courier_sent_for_the_stash_carries_on_what_it_already_holds() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let boots = bota_proto::ItemId(crate::game::ITEM_BOOTS);
    if let Some(bag) = world.inventory.get_mut(courier) {
        bag.slots[0] = Some(crate::game::ItemStack {
            id: boots,
            charges: 0,
            cooldown: 0,
            mute: 0,
            mode: None,
            bought_tick: 0,
            touched: false,
            owner: bota_proto::SlotId(0),
            for_sale: false,
        });
    }
    let home = crate::game::fountain_pos(world.map, bota_proto::Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(2500, 0);
    // The stash is empty, and the goods still go to the hero, not home.
    assert!(world.courier_take_stash(courier));
    for _ in 0..600 {
        world.step();
        if world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0)
        {
            break;
        }
    }
    assert_eq!(
        world.inventory.get(hero).expect("has a bag").slots[0].map(|held| held.id),
        Some(boots),
        "what it was already carrying reaches its owner"
    );
}

#[test]
fn what_an_owner_has_no_room_for_goes_back_to_the_stash() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    let branch = bota_proto::ItemId(crate::game::ITEM_IRON_BRANCH);
    let stack = crate::game::ItemStack {
        id: branch,
        charges: 1,
        cooldown: 0,
        mute: 0,
        mode: None,
        bought_tick: 0,
        touched: true,
        owner: bota_proto::SlotId(0),
        for_sale: false,
    };
    // Every slot the hero has is taken, so there is nowhere to hand it.
    if let Some(bag) = world.inventory.get_mut(hero) {
        for slot in bag.slots.iter_mut() {
            *slot = Some(stack);
        }
    }
    if let Some(bag) = world.inventory.get_mut(courier) {
        bag.slots[0] = Some(stack);
    }
    let home = crate::game::fountain_pos(world.map, bota_proto::Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos =
        home + bota_proto::Vec2::from_ints(1500, 0);
    assert!(world.courier_deliver(courier));
    for _ in 0..900 {
        world.step();
        if world.seats[0].stash.held().count() > 0 {
            break;
        }
    }
    assert_eq!(
        world.seats[0].stash.held().count(),
        1,
        "with nowhere to put it, it is carried back to the stash"
    );
    assert_eq!(
        world
            .inventory
            .get(courier)
            .expect("carries")
            .held()
            .count(),
        0,
        "and the courier is empty again"
    );
}
