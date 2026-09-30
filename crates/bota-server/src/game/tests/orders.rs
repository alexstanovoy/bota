//! Orders to units, and denying your own.

use crate::game::rules;
use crate::game::{Health, MELEE_CREEP, World};
use bota_proto::{Fixed, Team};

use super::support::*;

#[test]
fn an_order_at_something_a_side_cannot_see_is_refused() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(6800, 9216),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    world.seats[0].unit = Some(hero);
    let hidden = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(13200, 9216),
    );
    world.settle();
    crate::game::visibility_system(crate::game::SightCx {
        entities: &world.entities,
        transform: &world.transform,
        team: &world.team,
        kind: &world.kind,
        stats: &world.stats,
        ground: &world.ground,
        sight_block: &world.sight_block,
        visibility: &mut world.visibility,
        sight: &mut world.sight_scratch,
    });
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(hidden)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Err(bota_proto::RejectReason::UnknownTarget),
        "it is nowhere near and cannot be ordered at"
    );
    let near = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(6900, 9216),
    );
    world.settle();
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(near)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Ok(())
    );
}

#[test]
fn a_seat_with_no_body_standing_may_order_nothing() {
    let mut world = World::new();
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        bota_proto::Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    let order = bota_proto::Order::Move {
        target: bota_proto::Target::Pos(bota_proto::Vec2::ZERO),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Err(bota_proto::RejectReason::HeroDead)
    );
}

#[test]
fn an_attack_order_at_what_slipped_into_fog_walks_to_where_it_was_last_seen() {
    let (mut world, hero, mark) = hero_ordered_at_an_enemy(1000);
    let seen_at = world.transform.get(mark).expect("standing").pos;
    // It slips away through the fog, far past the hero's sight.
    if let Some(at) = world.transform.get_mut(mark) {
        at.pos = bota_proto::Vec2::from_ints(6000, 12000);
    }
    for _ in 0..240 {
        world.step();
        let now = world.transform.get(hero).expect("standing").pos;
        assert!(
            now.y.to_int() < 5400,
            "its path never bends after what its side cannot see: {now:?}"
        );
    }
    let stood = world.transform.get(hero).expect("standing").pos;
    assert!(
        stood.within(seen_at, Fixed::from_int(400)),
        "it walked to where the enemy was last seen: {stood:?}"
    );
}

#[test]
fn the_fight_rolls_onto_the_closest_when_the_ordered_target_falls() {
    let (mut world, hero, first) = hero_ordered_at_an_enemy(300);
    let second = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5570, 5150),
    );
    world.settle();
    for _ in 0..5 {
        world.step();
    }
    // The one it was set on falls to a blow.
    world.push_hit(Some(hero), first, 10_000, bota_proto::DamageKind::Pure);
    world.step();
    assert!(!world.alive(first), "the blow was fatal");
    assert!(
        matches!(
            world.orders.get(hero).map(|o| o.current),
            Some(crate::game::UnitOrder::AttackMove { .. })
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
    world.push_hit(Some(hero), mark, 10_000, bota_proto::DamageKind::Pure);
    world.step();
    assert!(!world.alive(mark), "the blow was fatal");
    assert!(
        matches!(
            world.orders.get(hero).map(|o| o.current),
            Some(crate::game::UnitOrder::AttackMove { .. })
        ),
        "the order degrades to fighting from where it fell"
    );
    for _ in 0..60 {
        world.step();
    }
    assert_eq!(world.target_of(hero), None, "with nobody around it waits");
    // The next one to come into acquisition is taken on unasked.
    let next = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5500, 5100),
    );
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
fn a_follow_at_what_slipped_into_fog_walks_to_where_it_was_last_seen() {
    let (mut world, hero, mark) = hero_following_an_enemy(1000);
    let seen_at = world.transform.get(mark).expect("standing").pos;
    if let Some(at) = world.transform.get_mut(mark) {
        at.pos = bota_proto::Vec2::from_ints(6000, 12000);
    }
    for _ in 0..240 {
        world.step();
        let now = world.transform.get(hero).expect("standing").pos;
        assert!(
            now.y.to_int() < 5400,
            "its path never bends after what its side cannot see: {now:?}"
        );
    }
    let stood = world.transform.get(hero).expect("standing").pos;
    assert!(
        stood.within(seen_at, Fixed::from_int(400)),
        "it walked to where the one followed was last seen: {stood:?}"
    );
}

#[test]
fn a_follow_ends_where_the_one_followed_fell() {
    let (mut world, hero, mark) = hero_following_an_enemy(1000);
    let theirs = world.transform.get(mark).expect("standing").pos;
    world.push_hit(None, mark, 10_000, bota_proto::DamageKind::Pure);
    world.step();
    assert!(!world.alive(mark), "the blow was fatal");
    assert!(
        matches!(
            world.orders.get(hero).map(|o| o.current),
            Some(crate::game::UnitOrder::Move { .. })
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
    let (mut world, hero, enemy) = hero_past_an_enemy(crate::game::UnitOrder::Move {
        pos: bota_proto::Vec2::from_ints(7000, 5000),
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
    let (mut world, hero, enemy) = hero_past_an_enemy(crate::game::UnitOrder::AttackMove {
        pos: bota_proto::Vec2::from_ints(7000, 5000),
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
    let (mut world, hero, enemy) = hero_past_an_enemy(crate::game::UnitOrder::Hold);
    // Out of reach, so anything that walked would walk.
    if let Some(at) = world.transform.get_mut(enemy) {
        at.pos = bota_proto::Vec2::from_ints(5500, 5000);
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
    let (mut world, hero, enemy) = hero_past_an_enemy(crate::game::UnitOrder::AttackMove {
        pos: bota_proto::Vec2::from_ints(7000, 5000),
    });
    world.step();
    assert_eq!(
        world.target_of(hero),
        Some(enemy),
        "walking to attack, it took the enemy on"
    );
    // The stop key.
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
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
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(own)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Ok(()),
        "the order may still be given: it is how creeps are shaken off"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order,
    }]);
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
    let max = world.stats.get(own).expect("settled").max_hp;
    // A shade under half of what it can hold.
    world.health.insert(
        own,
        Health {
            hp: Fixed {
                raw: max.raw * 49 / 100,
            },
        },
    );
    assert!(
        world.may_attack_on_order(hero, own),
        "worn down, it may be put out"
    );
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(own)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Ok(())
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order,
    }]);
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
    let max = world.stats.get(own).expect("settled").max_hp;
    world.health.insert(
        own,
        Health {
            hp: Fixed {
                raw: max.raw * 49 / 100,
            },
        },
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(own)),
        },
    }]);
    assert_eq!(world.target_of(hero), Some(own));
    // Mended back over the line.
    world.health.insert(own, Health { hp: max });
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
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats[0].unit = Some(hero);
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        Team::Radiant,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    world.settle();
    world.fill_pools(hero);
    world.fill_pools(tower);
    let max = world.stats.get(tower).expect("settled").max_hp;
    assert!(
        !world.may_attack_on_order(hero, tower),
        "standing tall, it is nobody to strike"
    );
    // A tenth is still too much; a shade under is not.
    world.health.insert(
        tower,
        Health {
            hp: Fixed { raw: max.raw / 10 },
        },
    );
    assert!(
        !world.may_attack_on_order(hero, tower),
        "exactly a tenth is not below a tenth"
    );
    world.health.insert(
        tower,
        Health {
            hp: Fixed {
                raw: max.raw * 9 / 100,
            },
        },
    );
    assert!(
        world.may_attack_on_order(hero, tower),
        "worn past it, it may be put out"
    );
    let order = bota_proto::Order::Attack {
        target: bota_proto::Target::Unit(crate::game::wire_id(tower)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &order),
        Ok(())
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order,
    }]);
    assert_eq!(world.target_of(hero), Some(tower));
}

#[test]
fn your_own_hero_is_never_struck_however_worn_down() {
    let mut world = World::new();
    let mine = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let theirs = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5100, 5000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
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
    world.seats.push(crate::game::Seat::new(
        bota_proto::SlotId(0),
        Team::Radiant,
        bota_proto::HeroId(0),
        0,
        rules::STASH_SLOTS,
    ));
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.seats[0].unit = Some(hero);
    // Well out of reach, and at full health so it cannot be put out yet.
    let ours = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(6500, 5000),
    );
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
    let apart = crate::game::isqrt64(
        now.distance_squared(world.transform.get(ours).expect("standing").pos),
    );
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
    let full = world.health.get(ours).expect("standing").hp;
    // Worn down past half, it may be put out after all.
    let max = world.stats.get(ours).expect("settled").max_hp;
    world.health.insert(
        ours,
        Health {
            hp: Fixed {
                raw: max.raw * 40 / 100,
            },
        },
    );
    for _ in 0..120 {
        world.step();
    }
    assert_eq!(
        world.target_of(hero),
        Some(ours),
        "and strikes the moment it may"
    );
    assert!(
        world.health.get(ours).map_or(Fixed::ZERO, |h| h.hp) < full,
        "it did put damage on it"
    );
}

#[test]
fn walking_at_an_ally_stops_where_the_bodies_meet() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let at = world.transform.get(hero).expect("standing").pos;
    let ally = world.spawn_unit(
        &crate::game::MELEE_CREEP,
        bota_proto::Team::Radiant,
        at + bota_proto::Vec2::from_ints(600, 0),
    );
    world.settle();
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(crate::game::wire_id(ally)),
        },
    }]);
    let mut seen: Vec<bota_proto::Vec2> = Vec::new();
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
    // Once it has arrived it stays arrived: what used to happen is that it
    // pressed into the body, was eased out, and walked in again for ever.
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
