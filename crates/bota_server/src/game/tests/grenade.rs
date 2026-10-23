//! Wind Lace and the Blood Grenade: how fast a hero walks with boots and a
//! lace, a grenade from the order to its last beat, and the shop's stock.

use bota_core::{ModifierKind, rules};

use crate::game::{Entity, Event, MatchConfig, World, wire_id};
use bota_proto::{
    AbilityId, CastView, DamageKind, EffectId, EventKind, Fixed, ItemId, ItemSlot, Order,
    RejectReason, StatusFlags, Target, Team, Vec2,
};

use super::support::*;

const BASE_SPEED: i32 = 305;

/// A match of two Shadow Fiends, Radiant in seat 0 and Dire in seat 1, both
/// at an empty spot `apart` units from each other, one tick on.
fn two_fiends(apart: i32) -> (World, Entity, Entity) {
    let cfg = MatchConfig {
        starting_gold_lost: Vec::new(),
        picks: fiend_against_fiend(),
        ..config()
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    let (ours, theirs) = (
        world.seats[0].unit.expect("stood up"),
        world.seats[1].unit.expect("stood up"),
    );
    let at = an_empty_spot(&world);
    world.transform.get_mut(ours).expect("hero").pos = at;
    world.transform.get_mut(theirs).expect("hero").pos = at + Vec2::from_ints(apart, 0);
    world.step();
    (world, ours, theirs)
}

fn magic_blows(events: &[Event], from: Entity, on: Entity) -> usize {
    events
        .iter()
        .filter(|event| {
            matches!(event.kind, EventKind::Damaged { source, target, kind: DamageKind::Magical, .. }
                if source == Some(wire_id(from)) && target == wire_id(on))
        })
        .count()
}

/// A stack of one item for seat 0 in one slot.
fn carry(world: &mut World, hero: Entity, at: usize, item: ItemId, charges: u8) {
    world.inventory.get_mut(hero).expect("bag").slots[at] = Some(fresh_stack(item, charges));
}

#[test]
fn boots_and_a_lace_add_up_and_a_second_of_either_adds_nothing() {
    let cases: [(&[ItemId], i32); 5] = [
        (&[], BASE_SPEED),
        (&[ItemId::WIND_LACE], BASE_SPEED + 15),
        (&[ItemId::BOOTS], BASE_SPEED + 45),
        (&[ItemId::BOOTS, ItemId::WIND_LACE], BASE_SPEED + 60),
        (
            &[
                ItemId::BOOTS,
                ItemId::BOOTS,
                ItemId::WIND_LACE,
                ItemId::WIND_LACE,
            ],
            BASE_SPEED + 60,
        ),
    ];
    for (carried, speed) in cases {
        let (mut world, hero, _) = two_fiends(600);
        for (at, item) in carried.iter().enumerate() {
            carry(&mut world, hero, at, *item, 0);
        }
        world.step();
        assert_eq!(
            shown(&world, Some(Team::Radiant), hero).move_speed,
            Fixed::from_int(speed),
            "carrying {carried:?}"
        );
    }
}

#[test]
fn a_grenade_in_its_cast_point_shows_a_cast_that_names_no_item() {
    // Dota's thrower plays a cast animation the other side sees, and it
    // tells no item.
    let (mut world, hero, enemy) = two_fiends(500);
    carry(&mut world, hero, 0, ItemId::BLOOD_GRENADE, 1);
    world.step();
    let aim = world.transform.get(enemy).expect("standing").pos;
    advance_validated(
        &mut world,
        None,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::Pos(aim),
        },
    );
    let casting = |world: &World| {
        shown(world, Some(Team::Dire), hero).statuses.bits & StatusFlags::CASTING != 0
    };
    for _ in 0..10 {
        if casting(&world) {
            break;
        }
        world.step();
    }
    assert!(casting(&world), "the thrower stands in the cast point");
    assert_eq!(
        shown(&world, Some(Team::Dire), hero).cast,
        Some(CastView {
            ability: None,
            goes_off_in: ticks_of(200),
        }),
        "a fifth of a second from its start"
    );
}

#[test]
fn a_thrown_grenade_flies_bursts_slows_and_burns_five_times_then_lifts() {
    let (mut world, hero, enemy) = two_fiends(500);
    carry(&mut world, hero, 0, ItemId::BLOOD_GRENADE, 1);
    world.step();
    let speed = shown(&world, Some(Team::Dire), enemy).move_speed;
    let hp = world.health.get(hero).expect("standing").hp;
    let aim = world.transform.get(enemy).expect("standing").pos;
    let mut events = advance_validated(
        &mut world,
        None,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::Pos(aim),
        },
    );
    let mut flew = 0;
    let mut tick_of_blows = Vec::new();
    for _ in 0..rules::GRENADE_DEBUFF_TICKS + 60 {
        let view = world.view(Team::Radiant);
        if view
            .projectiles
            .iter()
            .any(|shot| shot.ability == Some(AbilityId::THROW_GRENADE))
        {
            flew += 1;
        }
        if magic_blows(&events, hero, enemy) > 0 {
            tick_of_blows.push(world.tick);
        }
        events.clear();
        if tick_of_blows.len() == 1 {
            let them = shown(&world, Some(Team::Radiant), enemy);
            assert_eq!(
                i64::from(them.move_speed.raw),
                i64::from(speed.raw) * i64::from(100 - rules::GRENADE_SLOW_PCT) / 100,
                "slowed by 15% once it landed"
            );
            assert_ne!(them.statuses.bits & StatusFlags::SLOWED, 0);
            assert_ne!(them.statuses.bits & StatusFlags::DOT, 0);
            assert!(
                them.effects
                    .iter()
                    .any(|effect| effect.id == EffectId::BLOOD_GRENADE),
                "the debuff shows on the enemy"
            );
        }
        events.extend(world.advance(&[]));
    }
    assert!(
        (13..=15).contains(&flew),
        "500 units at 1100 a second is fourteen ticks in the air, not {flew}"
    );
    assert!(
        world.health.get(hero).expect("standing").hp <= hp - Fixed::from_int(74),
        "the throw costs its thrower 75 health"
    );
    assert_eq!(tick_of_blows.len(), 6, "the impact and five beats");
    let first = tick_of_blows[0];
    let beats: Vec<u32> = tick_of_blows[1..].iter().map(|tick| tick - first).collect();
    assert_eq!(
        beats,
        [30, 60, 90, 120, 150],
        "a beat on every whole second"
    );
    let them = shown(&world, Some(Team::Radiant), enemy);
    assert_eq!(them.move_speed, speed, "the slow lifts with the debuff");
    assert!(
        !them
            .effects
            .iter()
            .any(|effect| effect.id == EffectId::BLOOD_GRENADE)
    );
}

#[test]
fn a_grenade_never_takes_its_throwers_last_point_and_misses_what_stands_outside() {
    let (mut world, hero, enemy) = two_fiends(700);
    carry(&mut world, hero, 0, ItemId::BLOOD_GRENADE, 2);
    world.health.get_mut(hero).expect("standing").hp = Fixed::from_int(40);
    // Held where it stands, so it does not walk in to swing at the hero.
    put_on(&mut world, enemy, ModifierKind::Stunned, 300);
    let wide = world.transform.get(enemy).expect("standing").pos
        - Vec2::from_ints(rules::GRENADE_RADIUS + 60, 0);
    let mut events = advance_validated(
        &mut world,
        None,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::Pos(wide),
        },
    );
    while slot_of(&world, hero, 0).map(|stack| stack.charges) == Some(2) {
        events.extend(world.advance(&[]));
    }
    let left = world.health.get(hero).expect("standing").hp;
    assert!(
        left >= Fixed::ONE && left < Fixed::from_int(2),
        "the health cost leaves the last point, not {left:?}"
    );
    assert!(
        slot_of(&world, hero, 0).is_some_and(|stack| stack.charges == 1 && stack.cooldown > 0),
        "one of the two charges is spent, and the other waits"
    );
    for _ in 0..60 {
        events.extend(world.advance(&[]));
    }
    assert_eq!(
        magic_blows(&events, hero, enemy),
        0,
        "an enemy outside the radius is untouched"
    );
}

#[test]
fn a_side_buys_two_grenades_then_one_more_every_three_minutes() {
    let (mut world, hero) = a_hero_with_gold(5000);
    let grenade = ItemId::BLOOD_GRENADE;
    let shelf = |world: &World, team: Team| {
        world
            .view(team)
            .stock
            .into_iter()
            .find(|held| held.item == grenade)
            .map(|held| (held.left, held.max, held.restock_left))
    };
    assert_eq!(
        shelf(&world, Team::Radiant).map(|(left, max, _)| (left, max)),
        Some((2, 3))
    );
    for _ in 0..2 {
        advance_validated(&mut world, None, Order::Buy { item: grenade });
    }
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| (stack.id, stack.charges)),
        Some((grenade, 2)),
        "two stack in one slot"
    );
    reject_purchase_without_mutation(&mut world, hero, grenade, RejectReason::OutOfStock);
    let (_, _, wait) = shelf(&world, Team::Radiant).expect("a shelf");
    assert!(wait > 0 && wait < 180 * rules::TICKS_PER_SECOND);
    assert!(
        world
            .view(Team::Radiant)
            .stock
            .iter()
            .all(|held| held.team == Team::Radiant),
        "the other side's shelf is not shown"
    );
    assert_eq!(
        world
            .view_full()
            .stock
            .iter()
            .find(|held| held.team == Team::Dire && held.item == grenade)
            .map(|held| held.left),
        Some(2),
        "and is not emptied by this side"
    );
    for _ in 0..wait {
        world.advance(&[]);
    }
    assert_eq!(shelf(&world, Team::Radiant).map(|(left, ..)| left), Some(1));
    advance_validated(&mut world, None, Order::Buy { item: grenade });
    assert_eq!(
        world
            .inventory
            .get(hero)
            .expect("bag")
            .slots
            .iter()
            .flatten()
            .filter(|stack| stack.id == grenade)
            .map(|stack| u32::from(stack.charges))
            .sum::<u32>(),
        3,
        "the third goes to a slot of its own"
    );
}
