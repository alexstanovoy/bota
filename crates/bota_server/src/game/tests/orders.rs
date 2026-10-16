//! Orders to units, and denying your own.

use bota_core::{MELEE_CREEP, isqrt64, map_of, rules, tower_def};

use crate::game::{Command, Health, Seat, UnitOrder, World, wire_id};
use bota_proto::{
    AbilitySlot, DamageKind, Fixed, HeroId, ItemId, ItemSlot, MapId, Order, RejectReason, SlotId,
    Target, Team, Vec2,
};

use super::support::*;

#[test]
fn an_order_at_something_a_side_cannot_see_is_refused() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    seated_hero(
        &mut world,
        Vec2::from_ints(6800, 9216),
        HeroId::SHADOW_FIEND,
    );
    let hidden = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(13200, 9216));
    world.settle();
    let order = Order::Attack {
        target: Target::Unit(wire_id(hidden)),
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::UnknownTarget),
        "it is nowhere near and cannot be ordered at"
    );
    let near = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(6900, 9216));
    world.settle();
    let order = Order::Attack {
        target: Target::Unit(wire_id(near)),
    };
    assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
}

#[test]
fn a_use_or_cast_at_something_a_side_cannot_see_is_refused_like_a_dead_target() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let hero = seated_hero(&mut world, Vec2::from_ints(6800, 9216), HeroId::PUDGE);
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[3].level = 1;
    }
    hand_item(&mut world, hero, ItemId::HEALING_SALVE, 1);
    let hidden = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(13200, 9216));
    world.settle();
    world.fill_pools(hero);
    let target = Target::Unit(wire_id(hidden));
    for order in [
        Order::Use {
            slot: ItemSlot(0),
            target,
        },
        Order::Cast {
            slot: AbilitySlot(3),
            target,
        },
    ] {
        assert_eq!(
            world.validate_order(SlotId(0), None, &order),
            Err(RejectReason::UnknownTarget),
            "{order:?}"
        );
    }
}

#[test]
fn a_seat_with_no_body_standing_may_order_nothing() {
    let mut world = World::new();
    world.seats.push(Seat::new(
        SlotId(0),
        Team::Radiant,
        HeroId::SHADOW_FIEND,
        0,
        rules::STASH_SLOTS,
    ));
    let order = Order::Move {
        target: Target::Pos(Vec2::ZERO),
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::HeroDead)
    );
}

#[test]
fn an_attack_or_follow_at_what_slipped_into_fog_walks_to_where_it_was_last_seen() {
    for (order, (mut world, hero, mark)) in [
        ("attack", hero_ordered_at_an_enemy(1000)),
        ("follow", hero_following_an_enemy(1000)),
    ] {
        let seen_at = world.transform.get(mark).expect("standing").pos;
        // It slips away through the fog, far past the hero's sight.
        if let Some(at) = world.transform.get_mut(mark) {
            at.pos = Vec2::from_ints(6000, 12000);
        }
        for _ in 0..240 {
            world.step();
            let now = world.transform.get(hero).expect("standing").pos;
            assert!(
                now.y.to_int() < 5400,
                "{order}: its path never bends after what its side cannot see: {now:?}"
            );
        }
        let stood = world.transform.get(hero).expect("standing").pos;
        assert!(
            stood.within(seen_at, Fixed::from_int(400)),
            "{order}: it walked to where the enemy was last seen: {stood:?}"
        );
    }
}

#[test]
fn the_fight_rolls_onto_the_closest_when_the_ordered_target_falls() {
    let (mut world, hero, first) = hero_ordered_at_an_enemy(300);
    let second = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5570, 5150));
    world.settle();
    for _ in 0..5 {
        world.step();
    }
    // The one it was set on falls to a blow.
    world.push_hit(Some(hero), first, 10_000, DamageKind::Pure);
    world.step();
    assert!(!world.alive(first), "the blow was fatal");
    assert!(
        matches!(
            world.orders.get(hero).map(|o| o.current),
            Some(UnitOrder::AttackMove { .. })
        ),
        "the order degrades to fighting from where it fell"
    );
    world.step();
    assert_eq!(
        world.target_of(hero),
        Some(second),
        "and the fight carries itself onto the closest"
    );
}

#[test]
fn an_attack_order_whose_target_fell_keeps_the_hero_fighting_from_the_spot() {
    let (mut world, hero, mark) = hero_ordered_at_an_enemy(300);
    for _ in 0..5 {
        world.step();
    }
    world.push_hit(Some(hero), mark, 10_000, DamageKind::Pure);
    world.step();
    assert!(!world.alive(mark), "the blow was fatal");
    assert!(
        matches!(
            world.orders.get(hero).map(|o| o.current),
            Some(UnitOrder::AttackMove { .. })
        ),
        "the order degrades to fighting from where it fell"
    );
    for _ in 0..60 {
        world.step();
    }
    assert_eq!(world.target_of(hero), None, "with nobody around it waits");
    // The next one to come into acquisition is taken on unasked.
    let next = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5500, 5100));
    world.settle();
    world.step();
    assert_eq!(
        world.target_of(hero),
        Some(next),
        "the auto attack carries on"
    );
}

#[test]
fn a_follow_at_an_enemy_closes_and_never_swings() {
    let (mut world, hero, mark) = hero_following_an_enemy(1000);
    let full = world.health.get(mark).expect("standing").hp;
    for _ in 0..240 {
        world.step();
        assert_eq!(world.target_of(hero), None, "a follow takes nothing on");
    }
    assert_eq!(
        world.health.get(mark).expect("standing").hp,
        full,
        "the one followed was never struck"
    );
    let stood = world.transform.get(hero).expect("standing").pos;
    let theirs = world.transform.get(mark).expect("standing").pos;
    assert!(
        stood.within(theirs, Fixed::from_int(200)),
        "the follower closed until the bodies met: {stood:?}"
    );
}

#[test]
fn a_follow_ends_where_the_one_followed_fell() {
    let (mut world, hero, mark) = hero_following_an_enemy(1000);
    let theirs = world.transform.get(mark).expect("standing").pos;
    world.push_hit(None, mark, 10_000, DamageKind::Pure);
    world.step();
    assert!(!world.alive(mark), "the blow was fatal");
    assert!(
        matches!(
            world.orders.get(hero).map(|o| o.current),
            Some(UnitOrder::Move { .. })
        ),
        "the follow became a walk to where it fell"
    );
    for _ in 0..180 {
        world.step();
    }
    let stood = world.transform.get(hero).expect("standing").pos;
    assert!(
        stood.within(theirs, Fixed::from_int(400)),
        "and the walk ends there: {stood:?}"
    );
}

#[test]
fn a_hero_told_to_walk_walks_past_what_it_meets() {
    let (mut world, hero, enemy) = hero_past_an_enemy(UnitOrder::Move {
        pos: Vec2::from_ints(7000, 5000),
    });
    let was = world.health.get(enemy).expect("standing").hp;
    for _ in 0..120 {
        world.step();
    }
    assert_eq!(
        world.target_of(hero),
        None,
        "walking somewhere, it takes on nothing"
    );
    assert_eq!(
        world.health.get(enemy).expect("standing").hp,
        was,
        "and strikes nothing"
    );
    let now = world.transform.get(hero).expect("standing").pos;
    assert!(
        now.x.to_int() > 6000,
        "it kept walking where it was sent: {now:?}"
    );
}

#[test]
fn a_hero_told_to_walk_and_attack_stops_for_what_it_meets() {
    let (mut world, hero, enemy) = hero_past_an_enemy(UnitOrder::AttackMove {
        pos: Vec2::from_ints(7000, 5000),
    });
    let was = world.health.get(enemy).expect("standing").hp;
    for _ in 0..120 {
        world.step();
    }
    assert_eq!(world.target_of(hero), Some(enemy), "it took on what it met");
    assert!(
        world.health.get(enemy).expect("standing").hp < was,
        "and struck it"
    );
}

#[test]
fn a_hero_holding_comes_round_but_never_leaves_the_spot() {
    let (mut world, hero, enemy) = hero_past_an_enemy(UnitOrder::Hold);
    // Out of reach, so anything that walked would walk.
    if let Some(at) = world.transform.get_mut(enemy) {
        at.pos = Vec2::from_ints(5500, 5000);
    }
    let stood = world.transform.get(hero).expect("standing").pos;
    for _ in 0..60 {
        world.step();
    }
    assert_eq!(
        world.target_of(hero),
        Some(enemy),
        "holding, it still takes on what comes near"
    );
    assert_eq!(
        world.transform.get(hero).expect("standing").pos,
        stood,
        "but it does not go after it"
    );
}

#[test]
fn a_hero_told_to_stop_stands_and_takes_on_nothing() {
    let (mut world, hero, enemy) = hero_past_an_enemy(UnitOrder::AttackMove {
        pos: Vec2::from_ints(7000, 5000),
    });
    world.step();
    assert_eq!(
        world.target_of(hero),
        Some(enemy),
        "walking to attack, it took the enemy on"
    );
    stop_click(&mut world);
    assert_eq!(
        world.target_of(hero),
        None,
        "stopped, it gives up what it was on"
    );
    let stood = world.transform.get(hero).expect("standing").pos;
    let was = world.health.get(enemy).expect("standing").hp;
    for _ in 0..120 {
        world.step();
    }
    assert_eq!(world.target_of(hero), None, "and takes on nothing more");
    assert_eq!(
        world.health.get(enemy).expect("standing").hp,
        was,
        "so it strikes nothing"
    );
    assert_eq!(
        world.transform.get(hero).expect("standing").pos,
        stood,
        "and keeps the ground it was left on"
    );
}

#[test]
fn one_of_your_own_at_full_health_cannot_be_struck() {
    let (mut world, hero, own) = hero_and_own_creep();
    assert!(
        !world.may_attack_on_order(hero, own),
        "a creep at full health is nobody to strike"
    );
    let order = Order::Attack {
        target: Target::Unit(wire_id(own)),
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Ok(()),
        "the order may still be given: it is how creeps are shaken off"
    );
    attack_click(&mut world, own);
    let was = world.health.get(own).expect("standing").hp;
    for _ in 0..90 {
        world.step();
    }
    assert_eq!(
        world.target_of(hero),
        None,
        "it takes on nothing of its own"
    );
    assert_eq!(
        world.health.get(own).expect("standing").hp,
        was,
        "and strikes nothing"
    );
}

#[test]
fn one_of_your_own_worn_down_far_enough_may_be_put_out() {
    let (mut world, hero, own) = hero_and_own_creep();
    // A shade under half of what it can hold.
    wear_down(&mut world, own, 49);
    assert!(
        world.may_attack_on_order(hero, own),
        "worn down, it may be put out"
    );
    let order = Order::Attack {
        target: Target::Unit(wire_id(own)),
    };
    assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
    attack_click(&mut world, own);
    assert_eq!(world.target_of(hero), Some(own), "it takes it on");
    let was = world.health.get(own).expect("standing").hp;
    for _ in 0..90 {
        world.step();
    }
    let now = world
        .health
        .get(own)
        .map_or(Fixed::ZERO, |health| health.hp);
    assert!(now < was, "and strikes it");
}

#[test]
fn a_deny_is_given_up_when_the_creep_is_no_longer_worn_down() {
    let (mut world, hero, own) = hero_and_own_creep();
    wear_down(&mut world, own, 49);
    attack_click(&mut world, own);
    assert_eq!(world.target_of(hero), Some(own));
    // Mended back over the line.
    world.fill_pools(own);
    world.step();
    assert_eq!(
        world.target_of(hero),
        None,
        "back on its feet, it is nobody to strike again"
    );
}

#[test]
fn your_own_building_goes_only_at_a_tenth() {
    let mut world = World::new();
    let hero = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    let tower = world.spawn_unit(tower_def(1), Team::Radiant, Vec2::from_ints(5300, 5000));
    world.settle();
    world.fill_pools(hero);
    world.fill_pools(tower);
    assert!(
        !world.may_attack_on_order(hero, tower),
        "standing tall, it is nobody to strike"
    );
    // A tenth is still too much; a shade under is not.
    wear_down(&mut world, tower, 10);
    assert!(
        !world.may_attack_on_order(hero, tower),
        "exactly a tenth is not below a tenth"
    );
    wear_down(&mut world, tower, 9);
    assert!(
        world.may_attack_on_order(hero, tower),
        "worn past it, it may be put out"
    );
    let order = Order::Attack {
        target: Target::Unit(wire_id(tower)),
    };
    assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
    attack_click(&mut world, tower);
    assert_eq!(world.target_of(hero), Some(tower));
}

#[test]
fn your_own_hero_is_never_struck_however_worn_down() {
    let mut world = World::new();
    let mine = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    let theirs = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5100, 5000),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    world.health.insert(theirs, Health { hp: Fixed::ONE });
    assert!(
        !world.may_attack_on_order(mine, theirs),
        "one of your own heroes is nobody to strike, worn down or not"
    );
}

#[test]
fn an_attack_order_at_one_of_your_own_walks_you_to_it_and_waits() {
    let mut world = World::new();
    let hero = seated_hero(
        &mut world,
        Vec2::from_ints(5000, 5000),
        HeroId::SHADOW_FIEND,
    );
    // Well out of reach, and at full health so it cannot be put out yet.
    let ours = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(6500, 5000));
    world.settle();
    world.fill_pools(hero);
    world.fill_pools(ours);
    attack_click(&mut world, ours);
    let start = world.transform.get(hero).expect("standing").pos;
    for _ in 0..90 {
        world.step();
    }
    let now = world.transform.get(hero).expect("standing").pos;
    assert!(
        now.x > start.x,
        "it walks to what it was pointed at: {now:?} from {start:?}"
    );
    // Right up to it, not merely into reach: it has nothing to do from reach.
    let hulls = world.hull.get(hero).expect("has one").collision
        + world.hull.get(ours).expect("has one").collision;
    let apart = isqrt64(now.distance_squared(world.transform.get(ours).expect("standing").pos));
    assert!(
        apart < i64::from(rules::units(rules::HERO_ATTACK_RANGE).raw),
        "it came nearer than its reach: {apart}"
    );
    assert!(
        apart >= i64::from(hulls.raw) - i64::from(rules::units(4).raw),
        "and no nearer than the bodies allow"
    );
    assert_eq!(
        world.target_of(hero),
        None,
        "but takes nothing on while it cannot be struck"
    );
    // Worn down past half, it may be put out after all.
    wear_down(&mut world, ours, 40);
    let worn = world.health.get(ours).expect("standing").hp;
    for _ in 0..120 {
        world.step();
    }
    assert_eq!(
        world.target_of(hero),
        Some(ours),
        "and strikes the moment it may"
    );
    assert!(
        world.health.get(ours).map_or(Fixed::ZERO, |h| h.hp) < worn,
        "it did put damage on it"
    );
}

#[test]
fn walking_at_an_ally_stops_where_the_bodies_meet() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let at = world.transform.get(hero).expect("standing").pos;
    let ally = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at + Vec2::from_ints(600, 0));
    world.settle();
    attack_click(&mut world, ally);
    let mut seen: Vec<Vec2> = Vec::new();
    for _ in 0..240 {
        world.advance(&[]);
        seen.push(world.transform.get(hero).expect("standing").pos);
    }
    let theirs = world.transform.get(ally).expect("standing").pos;
    let hulls = world.hull.get(hero).expect("has one").collision
        + world.hull.get(ally).expect("has one").collision;
    let last = *seen.last().expect("walked");
    assert!(
        last.within(theirs, hulls + rules::units(rules::STEER_MARGIN * 2)),
        "it comes right up to what it was pointed at"
    );
    // Once it has arrived it stays arrived, not pressing into the body and
    // being eased out over and over.
    let settled = &seen[seen.len() - 60..];
    let drift = settled
        .iter()
        .map(|spot| {
            let (dx, dy) = (spot.x - last.x, spot.y - last.y);
            dx.raw.abs().max(dy.raw.abs())
        })
        .max()
        .expect("some ticks");
    assert!(
        drift < rules::units(24).raw,
        "and it stands there rather than circling"
    );
}

#[test]
fn an_attack_order_at_another_breaks_the_windup_off_and_one_at_the_same_lets_it_strike() {
    let attack = |world: &mut World, mark| {
        world.advance(&[Command {
            slot: SlotId(0),
            unit: None,
            order: Order::Attack {
                target: Target::Unit(wire_id(mark)),
            },
        }]);
    };
    let (mut world, hero, first) = hero_ordered_at_an_enemy(300);
    let second = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5300, 5100));
    world.settle();
    assert_eq!(swinging(&world, hero), Some(first), "the swing began");
    // The same target again: the windup runs on.
    attack(&mut world, first);
    assert_eq!(swinging(&world, hero), Some(first), "kept");
    // Another target: given up at once, as Dota's animation cancel, and begun at it.
    attack(&mut world, second);
    assert_eq!(
        swinging(&world, hero),
        Some(second),
        "begun again at the other"
    );
    let first_full = world.health.get(first).expect("standing").hp;
    let second_full = world.health.get(second).expect("standing").hp;
    while swinging(&world, hero) == Some(second) {
        world.advance(&[]);
    }
    assert_eq!(
        world.health.get(first).expect("standing").hp,
        first_full,
        "nothing struck the first"
    );
    for _ in 0..60 {
        world.advance(&[]);
    }
    assert!(
        world.health.get(second).expect("standing").hp < second_full,
        "the blow went to the second"
    );
}
