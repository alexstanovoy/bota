//! Couriers and their errands.

use bota_core::{MELEE_CREEP, fountain_pos, isqrt64, rules};

use crate::game::{BAG_SLOTS, Command, Entity, Errand, ItemStack, UnitOrder, World, wire_id};
use bota_proto::{
    AbilityId, AbilitySlot, DamageKind, EventKind, Fixed, ItemId, Order, RejectReason, SlotId,
    Target, Team, Vec2,
};

use super::support::*;

#[test]
fn a_courier_brought_down_comes_back_in_its_own_time() {
    let (mut world, _, courier) = a_match();
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
        Some(fountain_pos(world.map, Team::Radiant)),
        "at its own fountain"
    );
}

#[test]
fn a_courier_fetches_the_stash_hands_it_over_and_turns_for_home() {
    let (mut world, hero, courier) = a_match();
    let home = fountain_pos(world.map, Team::Radiant);
    let boots = ItemId::BOOTS;
    world.seats[0].stash.slots[0] = Some(fresh_stack(ItemId::BOOTS, 0));
    // Standing at the fountain, it takes what waits there on the next tick.
    assert!(world.courier_take_stash(courier));
    world.step();
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "the stash is empty"
    );
    assert_eq!(
        slot_of(&world, courier, 0).map(|held| held.id),
        Some(boots),
        "and the courier holds it"
    );
    world.transform.get_mut(hero).expect("standing").pos = home + Vec2::from_ints(900, 0);
    assert!(world.courier_deliver(courier));
    step_until(&mut world, 400, |world| {
        world.errand.get(courier) == Some(&Errand::None)
    });
    assert_eq!(
        slot_of(&world, hero, 0).map(|held| held.id),
        Some(boots),
        "what it carried is in its owner's hands"
    );
    assert_eq!(
        held(&world, courier),
        0,
        "and the courier carries nothing now"
    );
    assert!(
        matches!(
            world.orders.get(courier).map(|orders| orders.current),
            Some(UnitOrder::Move { pos }) if pos == home
        ),
        "and it turned for home on its own"
    );
}

#[test]
fn an_order_goes_to_the_unit_it_names_and_only_to_ones_this_seat_drives() {
    let (mut world, hero, courier) = a_match();
    let to = fountain_pos(world.map, Team::Radiant) + Vec2::from_ints(600, 0);
    let walk = Order::Move {
        target: Target::Pos(to),
    };
    // Naming nobody is the hero.
    assert_eq!(world.validate_order(SlotId(0), None, &walk), Ok(()));
    // Naming its own courier is allowed, and the order lands on the courier.
    let named = Some(wire_id(courier));
    assert_eq!(
        world.validate_order(SlotId(0), named, &walk),
        Ok(()),
        "a seat drives its own courier"
    );
    world.advance(&[Command {
        slot: SlotId(0),
        unit: named,
        order: walk,
    }]);
    assert!(
        matches!(
            world.orders.get(courier).map(|orders| orders.current),
            Some(UnitOrder::Move { pos }) if pos == to
        ),
        "the courier was told, not the hero"
    );
    assert!(
        !matches!(
            world.orders.get(hero).map(|orders| orders.current),
            Some(UnitOrder::Move { .. })
        ),
        "and the hero was left alone"
    );
    // Anything else is nobody this seat drives.
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(6000, 6000));
    world.settle();
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(creep)), &walk),
        Err(RejectReason::NotYourUnit),
        "a creep of its own side is still not its to drive"
    );
}

#[test]
fn a_courier_told_to_go_at_a_unit_follows_it() {
    let (mut world, _, courier) = a_match();
    let home = fountain_pos(world.map, Team::Radiant);
    // Something of its own side standing a way off, that then walks further.
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Radiant, home + Vec2::from_ints(700, 0));
    world.settle();
    world.advance(&[Command {
        slot: SlotId(0),
        unit: Some(wire_id(courier)),
        order: Order::Attack {
            target: Target::Unit(wire_id(mark)),
        },
    }]);
    for _ in 0..120 {
        world.step();
    }
    let near = |world: &World| {
        let at = world.transform.get(courier).expect("standing").pos;
        let to = world.transform.get(mark).expect("standing").pos;
        isqrt64(at.distance_squared(to)) <= i64::from(Fixed::from_int(150).raw)
    };
    assert!(near(&world), "it went to it");
    // Moved on, it is followed rather than left behind.
    world.transform.get_mut(mark).expect("standing").pos = home + Vec2::from_ints(700, 900);
    for _ in 0..200 {
        world.step();
    }
    assert!(near(&world), "and it keeps up when the mark moves");
}

#[test]
fn a_courier_at_the_fountain_reaches_the_stash_itself() {
    let (mut world, hero, courier) = a_match();
    let boots = ItemId::BOOTS;
    world.seats[0].stash.slots[0] = Some(fresh_stack(ItemId::BOOTS, 0));
    // The hero is out in the lane, so the stash is nothing to it.
    world.transform.get_mut(hero).expect("standing").pos =
        fountain_pos(world.map, Team::Radiant) + Vec2::from_ints(4000, 0);
    assert!(
        !world.move_item(SlotId(0), hero, BAG_SLOTS, 0),
        "out in the lane a hero cannot reach into the stash"
    );
    assert!(
        world.move_item(SlotId(0), courier, BAG_SLOTS, 0),
        "the courier at the fountain reaches it"
    );
    assert_eq!(
        slot_of(&world, courier, 0).map(|held| held.id),
        Some(boots),
        "and what waited there is in its hands"
    );
    assert!(
        world.seats[0].stash.slots[0].is_none(),
        "the stash is empty"
    );
    // And back again.
    assert!(world.move_item(SlotId(0), courier, 0, BAG_SLOTS));
    assert_eq!(
        world.seats[0].stash.slots[0].map(|held| held.id),
        Some(boots),
        "it goes back the same way"
    );
}

#[test]
fn courier_auxiliary_casts_keep_delivery_tracking_a_moving_owner() {
    for (slot, ability, cooldown) in [
        (2, AbilityId::BURST, rules::COURIER_BURST_COOLDOWN),
        (4, AbilityId::SHIELD, rules::COURIER_SHIELD_COOLDOWN),
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
        if ability == AbilityId::BURST {
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
fn taking_the_stash_carries_it_on_without_being_asked_twice() {
    let (mut world, hero, courier) = a_match();
    let home = fountain_pos(world.map, Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos = home + Vec2::from_ints(900, 0);
    world.seats[0].stash.slots[0] = Some(fresh_stack(ItemId::BOOTS, 0));
    assert!(world.courier_take_stash(courier));
    step_until(&mut world, 300, |world| held(world, hero) > 0);
    assert!(
        held(&world, hero) > 0,
        "one press fetched it and brought it"
    );
}

#[test]
fn an_errand_with_nothing_to_do_sends_the_courier_home() {
    let (mut world, _, courier) = a_match();
    let home = fountain_pos(world.map, Team::Radiant);
    world.transform.get_mut(courier).expect("standing").pos = home + Vec2::from_ints(1500, 0);
    // Nothing in the stash and nothing in its hands.
    assert!(world.courier_take_stash(courier));
    world.step();
    assert_eq!(
        world.errand.get(courier),
        Some(&Errand::GoingHome),
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
    let (mut world, hero, courier) = a_match();
    hand_item(&mut world, courier, ItemId::BOOTS, 0);
    world.bury(vec![(hero, None)], &mut Vec::new());
    assert!(world.courier_deliver(courier));
    step_until(&mut world, 300, |world| {
        world.seats[0].stash.held().count() > 0
    });
    assert!(
        world.seats[0].stash.held().count() > 0,
        "with nobody to hand to, it put it back in the stash"
    );
}

#[test]
fn a_shielded_courier_takes_nothing() {
    let (mut world, _, courier) = a_match();
    world.step();
    let full = world.health.get(courier).expect("standing").hp;
    assert!(world.courier_shield(courier));
    world.step();
    world.push_hit(None, courier, 100, DamageKind::Pure);
    world.step();
    assert_eq!(
        world.health.get(courier).map(|health| health.hp),
        Some(full),
        "nothing gets through while it holds"
    );
    for _ in 0..rules::COURIER_SHIELD_TICKS {
        world.step();
    }
    world.push_hit(None, courier, 100, DamageKind::Pure);
    world.step();
    assert!(
        world.health.get(courier).expect("standing").hp < full,
        "and once it lifts the courier is a courier again"
    );
}

#[test]
fn a_courier_keeps_its_load_through_death() {
    let (mut world, _hero) = a_hero_with_gold(0);
    let courier = the_courier(&world);
    world.inventory.get_mut(courier).expect("carries").slots[0] =
        Some(a_stack_of(ItemId::BOOTS, 0));
    world.bury(vec![(courier, None)], &mut Vec::new());
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
    assert_eq!(held(&world, back), 1, "with the load back aboard");
}

#[test]
fn a_courier_sent_for_the_stash_carries_on_what_it_already_holds() {
    let (mut world, hero, courier) = a_match();
    hand_item(&mut world, courier, ItemId::BOOTS, 0);
    let home = fountain_pos(world.map, Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos = home + Vec2::from_ints(2500, 0);
    // The stash is empty, and the goods still go to the hero, not home.
    assert!(world.courier_take_stash(courier));
    step_until(&mut world, 600, |world| held(world, hero) > 0);
    assert_eq!(
        slot_of(&world, hero, 0).map(|held| held.id),
        Some(ItemId::BOOTS),
        "what it was already carrying reaches its owner"
    );
}

#[test]
fn what_an_owner_has_no_room_for_goes_back_to_the_stash() {
    let (mut world, hero, courier) = a_match();
    let stack = ItemStack {
        charges: 1,
        ..a_stack_of(ItemId::IRON_BRANCH, 0)
    };
    // Every slot the hero has is taken, so there is nowhere to hand it.
    world
        .inventory
        .get_mut(hero)
        .expect("has a bag")
        .slots
        .fill(Some(stack));
    world.inventory.get_mut(courier).expect("carries").slots[0] = Some(stack);
    let home = fountain_pos(world.map, Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos = home + Vec2::from_ints(1500, 0);
    assert!(world.courier_deliver(courier));
    step_until(&mut world, 900, |world| {
        world.seats[0].stash.held().count() > 0
    });
    assert_eq!(
        world.seats[0].stash.held().count(),
        1,
        "with nowhere to put it, it is carried back to the stash"
    );
    assert_eq!(held(&world, courier), 0, "and the courier is empty again");
}

/// A match world, its hero and its courier, before any tick has run.
fn a_match() -> (World, Entity, Entity) {
    let world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    (world, hero, courier)
}

/// Steps a world up to `ticks` times, stopping on the tick `done` holds.
fn step_until(world: &mut World, ticks: u32, done: impl Fn(&World) -> bool) {
    for _ in 0..ticks {
        world.step();
        if done(world) {
            break;
        }
    }
}

/// How many stacks a unit carries.
fn held(world: &World, unit: Entity) -> usize {
    world.inventory.get(unit).expect("has a bag").held().count()
}
