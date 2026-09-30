//! The attack cycle and uphill misses.

use crate::game::rules;
use crate::game::{Health, MELEE_CREEP, World};
use bota_proto::{Fixed, Team};

use super::support::*;

#[test]
fn a_swing_waits_on_the_angle_it_is_looking_at() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    // Turned right away from it: in reach, seen, but not looked at.
    if let Some(at) = world.transform.get_mut(attacker) {
        at.facing = bota_proto::Angle { brads: 32768 };
    }
    swing_once(&mut world);
    assert_eq!(
        swinging(&world, attacker),
        None,
        "nothing begins while it is looking the other way"
    );
    // Looking straight at it.
    if let Some(at) = world.transform.get_mut(attacker) {
        at.facing = bota_proto::Angle { brads: 0 };
    }
    swing_once(&mut world);
    assert!(
        swinging(&world, attacker).is_some(),
        "and begins once it is"
    );
}

#[test]
fn a_swing_waits_on_being_able_to_see_at_all() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    if let Some(seen) = world.visibility.get_mut(mark) {
        seen.clear();
    }
    swing_once(&mut world);
    assert_eq!(
        swinging(&world, attacker),
        None,
        "what a side has no eyes on it does not swing at"
    );
}

#[test]
fn a_swing_waits_on_reach() {
    let (mut world, attacker, mark) = duel(600);
    world.set_target(attacker, mark);
    swing_once(&mut world);
    assert_eq!(swinging(&world, attacker), None, "too far to touch");
}

#[test]
fn a_swing_lands_on_whoever_it_began_against() {
    let (mut world, attacker, mark) = duel(100);
    let other = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    world.settle();
    world.set_target(attacker, mark);
    world.step();
    assert!(swinging(&world, attacker).is_some(), "the swing began");
    // Set on somebody else halfway through.
    world.set_target(attacker, other);
    let was = world.health.get(mark).expect("standing").hp;
    for _ in 0..ticks_of(rules::MELEE_CREEP_ATTACK_POINT) + 1 {
        world.step();
    }
    assert!(
        world.health.get(mark).expect("standing").hp < was,
        "and landed on the one it began against"
    );
}

#[test]
fn a_hero_told_to_attack_comes_round_and_closes() {
    let mut world = World::new();
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(4000, 5000),
    );
    world.settle();
    // Ordered at it, the way a player does.
    world.orders.insert(
        hero,
        crate::game::Orders {
            current: crate::game::UnitOrder::Attack {
                target: mark,
                last_seen: bota_proto::Vec2::from_ints(4000, 5000),
            },
            cooldown: 0,
            pending: None,
        },
    );
    let start = world.transform.get(hero).expect("standing").pos;
    for _ in 0..60 {
        world.step();
    }
    let now = world.transform.get(hero).expect("standing").pos;
    assert!(now.x < start.x, "it walked at what it was set on: {now:?}");
}

#[test]
fn a_swing_that_began_still_connects_a_little_past_reach() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(
        swinging(&world, attacker).is_some(),
        "the swing began in reach"
    );
    // Backs off by less than the leeway while the swing is under way.
    let just_out = rules::MELEE_CREEP_ATTACK_RANGE + 60;
    let was = world.health.get(mark).expect("standing").hp;
    for _ in 0..ticks_of(rules::MELEE_CREEP_ATTACK_POINT) + 1 {
        if let Some(at) = world.transform.get_mut(mark) {
            at.pos = bota_proto::Vec2::from_ints(5000 + just_out, 5000);
        }
        world.step();
    }
    assert!(
        world.health.get(mark).expect("standing").hp < was,
        "a step past reach does not shake it off"
    );
}

#[test]
fn a_swing_is_given_up_when_the_target_gets_away() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(
        swinging(&world, attacker).is_some(),
        "the swing began in reach"
    );
    let far = rules::MELEE_CREEP_ATTACK_RANGE + rules::ATTACK_RANGE_LEEWAY + 200;
    let was = world.health.get(mark).expect("standing").hp;
    if let Some(at) = world.transform.get_mut(mark) {
        at.pos = bota_proto::Vec2::from_ints(5000 + far, 5000);
    }
    world.step();
    assert_eq!(
        swinging(&world, attacker),
        None,
        "the swing is given up the moment it gets away"
    );
    assert_eq!(
        world.action.get(attacker).map(|a| a.attack_cooldown),
        Some(0),
        "and costs nothing, so the next one may start at once"
    );
    assert_eq!(
        world.health.get(mark).expect("standing").hp,
        was,
        "nothing was struck"
    );
}

#[test]
fn a_swing_is_given_up_when_the_target_falls() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(swinging(&world, attacker).is_some());
    world.health.insert(mark, Health { hp: Fixed::ZERO });
    world.step();
    assert_eq!(
        swinging(&world, attacker),
        None,
        "there is nothing left to strike"
    );
}

#[test]
fn a_swing_is_given_up_when_the_target_is_lost_from_sight() {
    let (mut world, attacker, mark) = duel(100);
    world.set_target(attacker, mark);
    world.step();
    assert!(swinging(&world, attacker).is_some());
    // Blinded to it, the way stepping into fog would.
    if let Some(seen) = world.visibility.get_mut(mark) {
        seen.clear();
    }
    swing_once(&mut world);
    assert_eq!(
        swinging(&world, attacker),
        None,
        "it does not finish a swing at what it can no longer see"
    );
}

#[test]
fn a_swing_costs_the_swinger_the_ground_it_stands_on() {
    let (mut world, hero, theirs, ours, _foe) = a_lane_with_a_hero(300);
    // Nothing else for it to fight, so it takes the hero and keeps after it.
    world.despawn(ours);
    world.transform.get_mut(hero).expect("hero").pos = bota_proto::Vec2::from_ints(5280, 5000);
    attack_click(&mut world, theirs);
    world.advance(&[]);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "the creep takes the one in front of it"
    );
    let before = gap_along_lane(&world, hero, theirs);
    for _ in 0..60 {
        world.advance(&[crate::game::Command {
            slot: bota_proto::SlotId(0),
            unit: None,
            order: bota_proto::Order::Move {
                target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(3000, 5000)),
            },
        }]);
    }
    let after = gap_along_lane(&world, hero, theirs);
    // The creep is the faster of the two: only the ticks it spends rooted in
    // its swings let the hero pull away at all.
    assert!(
        after > before + 200,
        "the hero should be pulling away: {before} then {after}"
    );
}

#[test]
fn an_order_to_break_off_gives_up_a_swing_that_has_not_landed() {
    let (mut world, hero, theirs, _ours, _foe) = a_lane_with_a_hero(150);
    attack_click(&mut world, theirs);
    for _ in 0..60 {
        if swinging(&world, hero).is_some() {
            break;
        }
        world.advance(&[]);
    }
    assert!(
        swinging(&world, hero).is_some(),
        "the hero should be mid-swing by now"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    let action = world.action.get(hero).copied().expect("acting");
    assert_eq!(
        action.state,
        crate::game::ActionState::Ready,
        "the swing is given up"
    );
    assert_eq!(action.attack_cooldown, 0, "and nothing was spent on it");
}

#[test]
fn an_order_after_a_swing_lands_does_not_hurry_the_next_one() {
    let (mut world, hero, theirs, _ours, _foe) = a_lane_with_a_hero(150);
    attack_click(&mut world, theirs);
    for _ in 0..120 {
        if recovering(&world, hero) {
            break;
        }
        world.advance(&[]);
    }
    let before = world
        .action
        .get(hero)
        .copied()
        .expect("acting")
        .attack_cooldown;
    assert!(
        before > 0,
        "the swing that landed spent the wait for the next"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    assert!(!recovering(&world, hero), "the recovery is cancelled");
    let gain = crate::game::attack_gain(world.stats.get(hero).expect("settled").attack_speed);
    assert_eq!(
        world.action.get(hero).expect("acting").attack_cooldown,
        before - gain,
        "but the wait for the next swing runs on"
    );
}

#[test]
fn ranged_attacks_can_miss_uphill_but_not_on_level_ground() {
    let ground = crate::game::Ground::of(crate::game::map_of(bota_proto::MapId(0)));
    let (source, level, uphill) = nearby_elevations(&ground);

    let level_damage =
        ranged_damage_after_ticks(source, level, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);
    let uphill_damage =
        ranged_damage_after_ticks(source, uphill, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);

    assert!(level_damage > 0, "the level-ground control must attack");
    assert!(uphill_damage < level_damage, "uphill attacks must miss");
    assert!(
        uphill_damage > level_damage / 2,
        "the uphill miss rate must stay near one quarter"
    );
}

#[test]
fn an_uphill_miss_is_told_of_and_a_level_shot_never_is() {
    let ground = crate::game::Ground::of(crate::game::map_of(bota_proto::MapId(0)));
    let (source, level, uphill) = nearby_elevations(&ground);
    let (level_landed, level_missed) =
        ranged_swings_after_ticks(source, level, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);
    let (uphill_landed, uphill_missed) =
        ranged_swings_after_ticks(source, uphill, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_TARGET);
    assert!(level_landed > 0, "the level-ground control lands");
    assert_eq!(level_missed, 0, "and nothing misses on level ground");
    assert!(uphill_missed > 0, "uphill, misses are told of");
    assert!(
        uphill_missed < uphill_landed,
        "and they are the smaller part: {uphill_missed} of {}",
        uphill_landed + uphill_missed
    );
}

#[test]
fn buildings_and_flying_attackers_are_exempt_from_uphill_misses() {
    let ground = crate::game::Ground::of(crate::game::map_of(bota_proto::MapId(0)));
    let (source, level, uphill) = nearby_elevations(&ground);

    let building_level =
        ranged_damage_after_ticks(source, level, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_BUILDING);
    let building_uphill =
        ranged_damage_after_ticks(source, uphill, &UPHILL_TEST_ATTACKER, &UPHILL_TEST_BUILDING);
    let flying_level = ranged_damage_after_ticks(
        source,
        level,
        &UPHILL_TEST_FLYING_ATTACKER,
        &UPHILL_TEST_TARGET,
    );
    let flying_uphill = ranged_damage_after_ticks(
        source,
        uphill,
        &UPHILL_TEST_FLYING_ATTACKER,
        &UPHILL_TEST_TARGET,
    );

    assert_eq!(building_uphill, building_level, "buildings do not evade");
    assert_eq!(flying_uphill, flying_level, "flying attacks do not miss");
}

#[test]
fn uphill_eligibility_uses_attacker_and_target_elevation_at_impact() {
    let ground = crate::game::Ground::of(crate::game::map_of(bota_proto::MapId(0)));
    let (low, _, high) = nearby_elevations(&ground);
    let low_tier = ground.tier(low);
    let high_tier = ground.tier(high);

    let moved_up = manual_projectile_damage(high, high, low_tier, false);
    let moved_down = manual_projectile_damage(low, high, high_tier, false);
    let pierced_down = manual_projectile_damage(low, high, high_tier, true);

    assert_eq!(
        moved_up, 512,
        "an attacker now level with its target does not miss"
    );
    assert!(
        moved_down < moved_up,
        "an attacker now below its target can miss"
    );
    assert!(
        moved_down > moved_up / 2,
        "the miss rate stays near one quarter"
    );
    assert_eq!(pierced_down, 512, "a shot that pierces never misses uphill");
}
