//! Couriers and their errands.

use bota_core::{
    MELEE_CREEP, ModifierKind, RANGED_CREEP, TOWERS, UnitDef, fountain_pos, hero_def,
    hero_spawn_pos, isqrt64, rules,
};

use crate::game::{
    BAG_SLOTS, Command, Entity, Errand, Inventory, ItemStack, MatchConfig, UnitOrder, World,
    wire_id,
};
use bota_proto::{
    AbilityId, AbilitySlot, DamageKind, EventKind, Fixed, HeroId, Inflictor, ItemId, Order,
    RejectReason, SlotId, StatusFlags, Target, Team, Vec2,
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
    assert_eq!(
        world.seats[0].courier,
        Some(courier),
        "the same courier lies dead"
    );
    assert!(world.seats[0].courier_left > 0, "and a wait has started");
    assert_eq!(world.pos(courier), None, "standing nowhere");
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(courier)), &deliver),
        Err(RejectReason::HeroDead),
        "a dead courier takes no order"
    );
    run(&mut world, rules::courier_respawn_ticks(1));
    let back = world.seats[0].courier.expect("it came back");
    assert_eq!(back, courier, "the same courier, as Dota's");
    assert_eq!(world.errand.get(back), Some(&Errand::None));
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(back)), &deliver),
        Ok(())
    );
    assert_eq!(
        world.transform.get(back).map(|at| at.pos),
        Some(hero_spawn_pos(world.map, Team::Radiant)),
        "by its own fountain"
    );
}

#[test]
fn a_courier_fetches_the_stash_hands_it_over_and_turns_for_home() {
    let (mut world, hero, courier) = a_match();
    let home = hero_spawn_pos(world.map, Team::Radiant);
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
    world.transform.get_mut(hero).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
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
        queue: false,
        seq: 0,
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
    let home = hero_spawn_pos(world.map, Team::Radiant);
    // Something of its own side standing a way off, that then walks further.
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Radiant, home + Vec2::from_ints(700, 0));
    world.settle();
    world.advance(&[Command {
        slot: SlotId(0),
        unit: Some(wire_id(courier)),
        order: Order::Attack {
            target: Target::Unit(wire_id(mark)),
        },
        queue: false,
        seq: 0,
    }]);
    run(&mut world, 120);
    let near = |world: &World| {
        let at = world.transform.get(courier).expect("standing").pos;
        let to = world.transform.get(mark).expect("standing").pos;
        isqrt64(at.distance_squared(to)) <= i64::from(Fixed::from_int(150).raw)
    };
    assert!(near(&world), "it went to it");
    // Moved on, it is followed rather than left behind.
    world.transform.get_mut(mark).expect("standing").pos = home + Vec2::from_ints(700, 900);
    run(&mut world, 200);
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
        let (mut world, hero, _) = courier_on_delivery();
        let courier = raise_owner(&mut world, rules::COURIER_SHIELD_LEVEL);
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
            // Still in its fountain's Rejuvenation, which sets its pace
            // outright: the burst adds nothing there.
            assert_eq!(speed, Fixed::from_int(rules::COURIER_FOUNTAIN_SPEED));
            assert_eq!(stats.move_speed, speed);
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
fn a_burst_or_a_shield_leaves_the_courier_walking_to_what_it_was_told_to_take() {
    // Dota's courier_burst and courier_shield are immediate casts: they go
    // off beside whatever the courier is doing and break none of it off.
    for slot in [2, 4] {
        let (mut world, _hero, _) = a_match();
        world.settle();
        let courier = raise_owner(&mut world, rules::COURIER_SHIELD_LEVEL);
        let from = world.transform.get(courier).expect("standing").pos;
        let lying = world.lay_loot(a_stack_of(ItemId::BOOTS, 0), from + Vec2::from_ints(600, 0));
        world.settle();
        advance_validated(
            &mut world,
            Some(courier),
            Order::Take {
                target: Target::Unit(wire_id(lying)),
            },
        );
        advance_validated(
            &mut world,
            Some(courier),
            Order::Cast {
                slot: AbilitySlot(slot),
                target: Target::None,
            },
        );
        step_until(&mut world, 300, |world| held(world, courier) > 0);
        assert_eq!(
            slot_of(&world, courier, 0).map(|stack| stack.id),
            Some(ItemId::BOOTS),
            "the cast in slot {slot} left the errand be"
        );
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
    world.transform.get_mut(hero).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
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
    let home = hero_spawn_pos(world.map, Team::Radiant);
    world.transform.get_mut(courier).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
    // Nothing in the stash and nothing in its hands.
    assert!(world.courier_take_stash(courier));
    world.step();
    assert_eq!(
        world.errand.get(courier),
        Some(&Errand::GoingHome),
        "with nothing to take it goes home"
    );
    run(&mut world, 400);
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
    let away = Vec2::from_ints(5000, 5000);
    world.transform.get_mut(courier).expect("standing").pos = away;
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, away + Vec2::from_ints(100, 0));
    world.settle();
    let full = world.health.get(courier).expect("standing").hp;
    assert!(world.courier_shield(courier));
    world.step();
    world.set_target(creep, courier);
    run(&mut world, rules::COURIER_SHIELD_TICKS - 1);
    assert_eq!(
        world.health.get(courier).map(|health| health.hp),
        Some(full),
        "nothing gets through while it holds"
    );
    assert!(
        blows_to_fell(&mut world, creep, courier) > 0,
        "and once it lifts the courier is a courier again"
    );
}

#[test]
fn a_courier_falls_to_one_melee_hero_swing_two_ranged_hero_missiles_or_six_other_blows() {
    let pudge = hero_def(HeroId::PUDGE).expect("a hero").unit;
    let fiend = hero_def(HeroId::SHADOW_FIEND).expect("a hero").unit;
    for (attacker, blows) in [
        (pudge, 1),
        (fiend, 2),
        (&MELEE_CREEP, 6),
        (&RANGED_CREEP, 6),
        (&TOWERS[0], 6),
    ] {
        let (mut world, courier, by) = a_courier_and(attacker);
        assert_eq!(
            world.stats.get(courier).map(|stats| stats.max_hp),
            Some(Fixed::from_int(rules::COURIER_HP)),
            "six hits of health"
        );
        assert_eq!(
            blows_to_fell(&mut world, by, courier),
            blows,
            "{:?} brings it down in {blows}",
            attacker.kind
        );
    }
}

/// What a side is shown of an enemy courier is its health in hits, the
/// numbers a front end draws its bar's blocks from: six, and three gone with
/// each missile of a ranged hero its seat sends at it.
#[test]
fn a_courier_a_seat_sends_its_ranged_hero_at_shows_six_hits_and_loses_three_a_missile() {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let fiend = seated_hero(&mut world, at, HeroId::SHADOW_FIEND);
    let courier = world.spawn_courier(
        Team::Dire,
        at + Vec2::from_ints(300, 0),
        SlotId(1),
        Inventory::empty(6),
    );
    world.settle();
    world.fill_pools(fiend);
    world.step();
    let health = |world: &World| {
        let unit = shown(world, Some(Team::Radiant), courier);
        (unit.hp, unit.max_hp)
    };
    let full = rules::COURIER_HP;
    assert_eq!(health(&world), (full, full), "six hits of health shown");
    attack_click(&mut world, courier);
    let mut shown_after = Vec::new();
    for _ in 0..(10 * rules::TICKS_PER_SECOND) {
        world.step();
        if !world.alive(courier) {
            break;
        }
        let (hp, max_hp) = health(&world);
        assert_eq!(max_hp, full);
        if hp < full && shown_after.last() != Some(&hp) {
            shown_after.push(hp);
        }
    }
    assert!(!world.alive(courier), "two missiles bring it down");
    assert_eq!(shown_after, vec![full - 3], "half of it after the first");
}

#[test]
fn a_spell_takes_nothing_of_a_courier_and_neither_does_damage_besides_a_swing() {
    let (mut world, courier, fiend) =
        a_courier_and(hero_def(HeroId::SHADOW_FIEND).expect("a hero").unit);
    let full = world.health.get(courier).expect("standing").hp;
    let raze = Some(Inflictor::Ability(AbilityId::RAZE_NEAR));
    world.push_hit(Some(fiend), courier, 1000, DamageKind::Magical, raze);
    world.push_hit(Some(fiend), courier, 1000, DamageKind::Pure, None);
    world.step();
    assert_eq!(
        world.health.get(courier).map(|health| health.hp),
        Some(full),
        "health counted in hits answers to attacks alone"
    );
}

#[test]
fn a_courier_keeps_its_load_through_death() {
    let (mut world, _hero) = a_hero_with_gold(0);
    let courier = the_courier(&world);
    world.inventory.get_mut(courier).expect("carries").slots[0] =
        Some(a_stack_of(ItemId::BOOTS, 0));
    world.bury(vec![(courier, None)], &mut Vec::new());
    assert_eq!(
        held(&world, courier),
        1,
        "the load stays aboard the fallen courier"
    );
    run(&mut world, rules::courier_respawn_ticks(1) + 2);
    let back = world.seats[0].courier.expect("stands again");
    assert_eq!(back, courier, "the same courier");
    assert_eq!(held(&world, back), 1, "with its load");
}

#[test]
fn a_courier_sent_for_the_stash_carries_on_what_it_already_holds() {
    let (mut world, hero, courier) = a_match();
    hand_item(&mut world, courier, ItemId::BOOTS, 0);
    world.transform.get_mut(hero).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
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
    world.transform.get_mut(hero).expect("standing").pos = rules::DEMO_LANE_CORNERS[1];
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
/// A Radiant courier on open ground and a Dire unit of a kind beside it,
/// settled and one tick on.
fn a_courier_and(attacker: &'static UnitDef) -> (World, Entity, Entity) {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let courier = world.spawn_courier(Team::Radiant, at, SlotId(0), Inventory::empty(6));
    let by = world.spawn_unit(attacker, Team::Dire, at + Vec2::from_ints(100, 0));
    world.settle();
    world.step();
    (world, courier, by)
}

/// The blows one unit lands before another falls, swinging at it from where
/// it stands.
fn blows_to_fell(world: &mut World, from: Entity, on: Entity) -> u32 {
    world.set_target(from, on);
    let mut blows = 0;
    let mut had = world.health.get(on).expect("standing").hp;
    for _ in 0..1200 {
        world.step();
        let now = world.health.get(on).map_or(Fixed::ZERO, |health| health.hp);
        if now < had {
            blows += 1;
            had = now;
        }
        if !world.alive(on) {
            return blows;
        }
    }
    panic!("it never fell")
}

fn a_match() -> (World, Entity, Entity) {
    let (world, hero) = match_hero();
    let courier = the_courier(&world);
    (world, hero, courier)
}

/// Carries a courier out of its fountain's reach and on until the
/// Rejuvenation it wore there has lingered out: its pace is its own again.
fn leave_the_fountain(world: &mut World, courier: Entity) {
    world.transform.get_mut(courier).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
    run(world, rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
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

fn held(world: &World, unit: Entity) -> usize {
    world.inventory.get(unit).expect("has a bag").held().count()
}

#[test]
fn a_courier_walks_at_290_and_ten_more_a_level_of_its_owner() {
    let (mut world, _, courier) = a_match();
    world.settle();
    let pace = |world: &World, courier| {
        world
            .stats
            .get(courier)
            .map(|stats| stats.move_speed.to_int())
    };
    assert_eq!(pace(&world, courier), Some(290), "Liquipedia's level 1");
    assert_eq!(shown(&world, Some(Team::Radiant), courier).level, 1);
    let courier = raise_owner(&mut world, 3);
    assert_eq!(pace(&world, courier), Some(310));
    assert!(
        !world.stats.get(courier).expect("settled").flies,
        "still on foot"
    );
    let courier = raise_owner(&mut world, 10);
    assert_eq!(pace(&world, courier), Some(380), "Liquipedia's level 10");
    assert_eq!(
        shown(&world, Some(Team::Radiant), courier).level,
        10,
        "and it shows its owner's level"
    );
}

#[test]
fn a_walking_courier_goes_round_what_closes_the_ground() {
    let hero_at = Vec2::from_ints(6000, 9216);
    let far = Vec2::from_ints(9000, 9216);
    let mut world = a_world_with_a_wall(hero_at, far);
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    world.transform.get_mut(hero).expect("standing").pos = far;
    world.transform.get_mut(courier).expect("walks").pos = hero_at;
    world.forget_walk(courier);
    hand_item(&mut world, courier, ItemId::BOOTS, 0);
    assert!(world.courier_deliver(courier));
    let mut widest = 0;
    for _ in 0..900 {
        world.step();
        let now = world.transform.get(courier).expect("walks").pos;
        assert!(world.clearance.walkable(now), "never on closed ground");
        widest = widest.max((now.y.raw - hero_at.y.raw).abs());
        if held(&world, hero) > 0 {
            break;
        }
    }
    assert!(held(&world, hero) > 0, "it got there");
    assert!(widest > rules::units(600).raw, "on foot, round the wall");
}

#[test]
fn at_the_flying_level_the_courier_takes_flight_and_every_attack_at_it_is_disjointed() {
    let (mut world, _, courier) = a_match();
    let away = rules::DEMO_LANE_CORNERS[0];
    world.transform.get_mut(courier).expect("standing").pos = away;
    hand_item(&mut world, courier, ItemId::BOOTS, 0);
    // A tower and a hero: neither marches off on a lane of its own.
    let archer = world.spawn_unit(&TOWERS[0], Team::Dire, away + Vec2::from_ints(250, 0));
    world.settle();
    world.set_target(archer, courier);
    let in_the_air = |world: &World| {
        world.entities.iter().any(|e| {
            world
                .projectile
                .get(e)
                .is_some_and(|shot| shot.target == courier)
        })
    };
    step_until(&mut world, 300, in_the_air);
    // A melee hero stands up beside it and winds a swing up while the missile
    // flies.
    let pudge = hero_def(HeroId::PUDGE).expect("a hero").unit;
    let brute = world.spawn_unit(pudge, Team::Dire, away - Vec2::from_ints(100, 0));
    world.settle();
    world.set_target(brute, courier);
    step_until(&mut world, 30, |world| {
        swinging(world, brute) == Some(courier)
    });
    assert!(in_the_air(&world), "a missile is in the air");
    assert_eq!(
        swinging(&world, brute),
        Some(courier),
        "and a swing winds up"
    );
    let hp = world.health.get(courier).expect("standing").hp;
    let flyer = raise_owner(&mut world, rules::COURIER_FLYING_LEVEL);
    assert_eq!(flyer, courier, "the same courier, its id kept");
    assert!(!in_the_air(&world), "the missile had nobody to reach");
    assert_eq!(
        swinging(&world, brute),
        None,
        "and the swing was broken off"
    );
    assert_eq!(world.pos(courier), Some(away));
    assert_eq!(held(&world, courier), 1, "with its load");
    assert!(world.stats.get(courier).expect("settled").flies);
    assert!(
        shown(&world, Some(Team::Radiant), courier)
            .effects
            .iter()
            .any(|effect| effect.id == bota_proto::EffectId::FLYING),
        "wearing its flight"
    );
    world.set_order(archer, UnitOrder::Stand);
    world.set_order(brute, UnitOrder::Stand);
    world.step();
    assert_eq!(
        world.health.get(courier).map(|health| health.hp),
        Some(hp),
        "nothing aimed at it before it took flight landed"
    );
}

#[test]
fn burst_opens_at_owner_level_ten_and_shield_at_twenty_and_no_point_buys_them() {
    let (mut world, _, _) = a_match();
    world.settle();
    let burst = Order::Cast {
        slot: AbilitySlot(2),
        target: Target::None,
    };
    let shield = Order::Cast {
        slot: AbilitySlot(4),
        target: Target::None,
    };
    let learn = Order::Learn {
        slot: AbilitySlot(2),
    };
    let at = |world: &World, courier, slot: usize| {
        shown(world, Some(Team::Radiant), courier).abilities[slot]
    };
    let courier = raise_owner(&mut world, 9);
    for (order, slot) in [(burst, 2), (shield, 4)] {
        assert_eq!(
            world.validate_order(SlotId(0), Some(wire_id(courier)), &order),
            Err(RejectReason::NotLearned)
        );
        assert_eq!(at(&world, courier, slot).level, 0, "shown shut");
        assert!(
            !at(&world, courier, slot).can_level,
            "and no point opens it"
        );
    }
    world.advance(&[Command {
        slot: SlotId(0),
        unit: Some(wire_id(courier)),
        order: learn,
        queue: false,
        seq: 0,
    }]);
    assert_eq!(at(&world, courier, 2).level, 0, "a point bought nothing");
    let courier = raise_owner(&mut world, rules::COURIER_BURST_LEVEL);
    assert_eq!(at(&world, courier, 2).level, 1);
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(courier)), &burst),
        Ok(())
    );
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(courier)), &shield),
        Err(RejectReason::NotLearned)
    );
    let courier = raise_owner(&mut world, rules::COURIER_SHIELD_LEVEL);
    assert_eq!(
        world.validate_order(SlotId(0), Some(wire_id(courier)), &shield),
        Ok(())
    );
}

#[test]
fn a_fallen_courier_pays_every_enemy_25_and_5_a_level_and_waits_45_s_and_5_a_level() {
    for (level, bounty, wait) in [(1, 25, 50), (10, 70, 95)] {
        let cfg = MatchConfig {
            starting_gold_lost: Vec::new(),
            economy_events: Vec::new(),
            picks: fiend_against_fiend(),
            ..config()
        };
        let mut world = World::for_match(&cfg, cfg.rng());
        world.settle();
        let courier = if level > 1 {
            raise_owner(&mut world, level)
        } else {
            the_courier(&world)
        };
        let killer = world.seats[1].unit.expect("stood up");
        let (ours, theirs) = (world.seats[0].gold, world.seats[1].gold);
        world.bury(vec![(courier, Some(killer))], &mut Vec::new());
        assert_eq!(world.seats[1].gold - theirs, bounty, "level {level}");
        assert_eq!(world.seats[0].gold, ours, "its own side is paid nothing");
        world.step();
        assert_eq!(
            world.seats[0].courier_left,
            wait * rules::TICKS_PER_SECOND - 1,
            "level {level}"
        );
    }
}

#[test]
fn a_burst_adds_half_the_pace_and_no_slow_takes_from_it_while_it_holds() {
    let (mut world, _, _) = a_match();
    world.settle();
    let courier = raise_owner(&mut world, rules::COURIER_BURST_LEVEL);
    leave_the_fountain(&mut world, courier);
    put_on(&mut world, courier, ModifierKind::Slowed { pct: 30 }, 600);
    world.settle();
    let pace = |world: &World| {
        world
            .stats
            .get(courier)
            .map(|stats| stats.move_speed.to_int())
    };
    assert_eq!(
        pace(&world),
        Some(380 * 70 / 100),
        "slowed before the burst"
    );
    advance_validated(
        &mut world,
        Some(courier),
        Order::Cast {
            slot: AbilitySlot(2),
            target: Target::None,
        },
    );
    world.advance(&[]);
    assert_eq!(
        pace(&world),
        Some(570),
        "hasted: 380 and half, slow or no slow"
    );
    assert!(
        pace(&world) > Some(rules::MAX_MOVE_SPEED),
        "past 550: a courier's pace knows no upper limit"
    );
    run(&mut world, rules::COURIER_BURST_TICKS);
    assert_eq!(
        pace(&world),
        Some(380 * 70 / 100),
        "slowed again once it lifts"
    );
}

#[test]
fn a_courier_in_its_fountains_rejuvenation_takes_nothing_and_is_hit_again_once_it_lifts() {
    let (mut world, _, courier) = a_match();
    world.settle();
    world.step();
    let swing = |world: &mut World| {
        world.hits.push_back(crate::game::Hit {
            source: None,
            inflictor: None,
            target: courier,
            amount: 1,
            kind: DamageKind::Physical,
            damage_amp_bp: rules::NOMINAL_BP,
            attack: true,
            striker: Some(bota_core::Striker::Other),
            pierces: true,
            uphill: false,
            effect: crate::game::HitEffect::None,
        });
        world.step();
    };
    let hp = |world: &World| world.health.get(courier).map(|health| health.hp);
    assert!(carries(
        &world,
        courier,
        ModifierKind::Fountain {
            hp_pct: rules::FOUNTAIN_HEAL_HP_PCT,
            mana_pct: rules::FOUNTAIN_HEAL_MANA_PCT,
        }
    ));
    assert!(
        shown(&world, Some(Team::Radiant), courier).statuses.bits & StatusFlags::INVULNERABLE != 0,
        "shown invulnerable by its fountain"
    );
    swing(&mut world);
    assert_eq!(
        hp(&world),
        Some(Fixed::from_int(rules::COURIER_HP)),
        "nothing lands"
    );
    world.transform.get_mut(courier).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
    run(&mut world, rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
    swing(&mut world);
    assert_eq!(
        hp(&world),
        Some(Fixed::from_int(rules::COURIER_HP - 1)),
        "out of the aura once it has lingered, a blow lands"
    );
}

/// Checked in the Dota 2 client (7.41, 2026-10-24): a courier at its fountain
/// moves at 550 ("courier moves faster" in its Rejuvenation buff), and keeps
/// it the 3 s the buff lingers after it leaves; then its own pace again.
#[test]
fn a_courier_moves_at_550_in_its_fountains_rejuvenation_and_3_s_after_it_leaves() {
    let (mut world, _, courier) = a_match();
    world.settle();
    world.step();
    let pace = |world: &World| {
        world
            .stats
            .get(courier)
            .map(|stats| stats.move_speed.to_int())
    };
    assert_eq!(
        pace(&world),
        Some(rules::COURIER_FOUNTAIN_SPEED),
        "at its fountain"
    );
    let from = world.pos(courier).expect("standing");
    let away = from + Vec2::from_ints(2000, 0);
    advance_validated(
        &mut world,
        Some(courier),
        Order::Move {
            target: Target::Pos(away),
        },
    );
    run(&mut world, rules::TICKS_PER_SECOND);
    let walked = isqrt64(world.pos(courier).expect("standing").distance_squared(from))
        / i64::from(Fixed::from_int(1).raw);
    assert!(
        (500..=570).contains(&walked),
        "it walks at 550: {walked} in the order's tick and a second"
    );
    world.transform.get_mut(courier).expect("standing").pos = rules::DEMO_LANE_CORNERS[0];
    run(&mut world, rules::FOUNTAIN_HEAL_LINGER_TICKS - 1);
    assert_eq!(
        pace(&world),
        Some(rules::COURIER_FOUNTAIN_SPEED),
        "while it lingers"
    );
    run(&mut world, 2);
    assert_eq!(
        pace(&world),
        Some(rules::COURIER_MOVE_SPEED),
        "then its own"
    );
}
