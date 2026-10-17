//! Towers at Dota 7.41f: what they are made of, how fast they shoot, what
//! their fall pays and how often they let go.

use bota_core::{MELEE_CREEP, UnitDef, rules, tower_def};

use crate::game::World;
use bota_proto::{Fixed, Team, UnitKind, Vec2};

use super::support::*;

/// A creep that bears no armor and never falls.
const TARGET: UnitDef = UnitDef {
    max_hp: 30_000,
    damage: 0,
    armor: 0,
    hp_regen: Fixed::ZERO,
    move_speed: 0,
    ..MELEE_CREEP
};

#[test]
fn every_tier_stands_and_shoots_as_in_seven_forty_one() {
    // Liquipedia, Tower (Tier 1) to (Tier 4): health, armor and damage
    // range; every tier shoots at attack speed 110 on a 0.9 s base, so a
    // shot every 0.818 s, 36 or 37 of them in 30 seconds.
    let tiers = [
        (1, 1800, 12, (88, 92)),
        (2, 2500, 16, (170, 174)),
        (3, 2500, 16, (170, 174)),
        (4, 2600, 21, (170, 174)),
    ];
    for (tier, hp, armor, (low, high)) in tiers {
        let mut world = World::new();
        let tower = world.spawn_unit(tower_def(tier), Team::Radiant, Vec2::from_ints(1500, 1500));
        let on = world.spawn_unit(&TARGET, Team::Dire, Vec2::from_ints(1900, 1500));
        world.settle();
        world.fill_pools(on);
        let stats = world.stats.get(tower).expect("settled");
        assert_eq!(world.kind.get(tower), Some(&UnitKind::Tower));
        assert_eq!(
            (stats.max_hp, stats.armor, stats.attack_speed),
            (Fixed::from_int(hp), Fixed::from_int(armor), 110),
            "tier {tier}"
        );
        world.set_target(tower, on);
        let mut shots = Vec::new();
        let mut before = world.health.get(on).expect("standing").hp;
        let mut first = None;
        while first.is_none_or(|at| world.tick < at + 900) {
            world.step();
            let now = world.health.get(on).expect("standing").hp;
            if now < before {
                first.get_or_insert(world.tick);
                shots.push((before - now).to_int());
            }
            before = now;
            assert!(world.tick < 2_000, "tier {tier}: the tower shoots");
        }
        // The shot landing on the first tick counted opens the window.
        let in_window = shots.len() - 1;
        assert!(
            (36..=37).contains(&in_window),
            "tier {tier}: {in_window} shots in 30 seconds"
        );
        assert!(
            shots.iter().all(|shot| (low..=high).contains(shot)),
            "tier {tier}: {shots:?}"
        );
        assert!(
            shots.contains(&low) && shots.contains(&high),
            "tier {tier}: {shots:?}"
        );
    }
}

#[test]
fn a_fallen_tower_pays_its_side_and_its_striker_and_a_deny_pays_both_sides_half() {
    // Liquipedia, Tower (Tier 1): last hit 110-130 on top of a 90 team
    // bounty to every hero of the side; denying grants 45 to both teams.
    let range = rules::TOWER_TIER_BOUNTY[0];
    let team = rules::TOWER_TIER_TEAM_BOUNTY[0];
    let (mut world, _) = a_tower_and_three_seats();
    let striker = world.seats[0].unit.expect("seated");
    let mut seen = Vec::new();
    for _ in 0..40 {
        let before: Vec<i32> = world.seats.iter().map(|seat| seat.gold).collect();
        let tower = world.spawn_unit(tower_def(1), Team::Dire, Vec2::from_ints(5000, 5000));
        world.settle();
        world.health.insert(tower, health(0));
        let mut events = Vec::new();
        world.bury(vec![(tower, Some(striker))], &mut events);
        let gold: Vec<i32> = world
            .seats
            .iter()
            .zip(&before)
            .map(|(seat, was)| seat.gold - was)
            .collect();
        let last_hit = gold[0] - team;
        assert!((range.min..=range.max).contains(&last_hit), "{gold:?}");
        assert_eq!(paid(&events), Some(gold[0]), "the event tells both");
        assert_eq!(gold[1], team, "an ally anywhere gets the team bounty");
        assert_eq!(gold[2], 0, "the side that lost it gets nothing");
        seen.push(last_hit);
    }
    assert!(seen.iter().any(|&g| g != seen[0]), "the last hit is drawn");
    let (mut world, tower) = a_tower_and_three_seats();
    world.health.insert(tower, health(0));
    let denier = world.seats[2].unit.expect("seated");
    let mut events = Vec::new();
    world.bury(vec![(tower, Some(denier))], &mut events);
    let gold: Vec<i32> = world.seats.iter().map(|seat| seat.gold).collect();
    assert_eq!(gold, vec![team / 2; 3], "a deny pays every seat half");
    assert_eq!(paid(&events), Some(team / 2));
}

/// A Dire tier-one tower, two Radiant seats (the first beside it, the
/// second across the map) and the Dire seat that owns it, all with no gold.
fn a_tower_and_three_seats() -> (World, crate::game::Entity) {
    let mut world = World::new();
    let tower = world.spawn_unit(tower_def(1), Team::Dire, Vec2::from_ints(5000, 5000));
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(5300, 5000));
    seat_hero(
        &mut world,
        1,
        Team::Radiant,
        Vec2::from_ints(15_000, 15_000),
    );
    seat_hero(&mut world, 2, Team::Dire, Vec2::from_ints(4700, 5000));
    world.settle();
    world.health.insert(tower, health(0));
    (world, tower)
}

#[test]
fn a_tower_lets_go_of_a_hero_at_most_once_every_two_and_a_half_seconds() {
    // Liquipedia, Buildings, "Manual de-aggro": an attack order at an
    // allied unit makes the tower choose again, "only once every 2.5
    // seconds this way".
    let mut world = World::new();
    let tower = world.spawn_unit(tower_def(1), Team::Dire, Vec2::from_ints(5000, 5000));
    let own = world.spawn_unit(&TARGET, Team::Radiant, Vec2::from_ints(5300, 5000));
    let hero = seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(5500, 5000));
    world.settle();
    let shot_at_then_shed = |world: &mut World| {
        world.set_target(tower, hero);
        attack_click(world, own);
        stop_click(world);
        world.target_of(tower)
    };
    assert_eq!(
        shot_at_then_shed(&mut world),
        Some(own),
        "the first click sheds it"
    );
    assert_eq!(
        shot_at_then_shed(&mut world),
        Some(hero),
        "a second click inside 2.5 s does not"
    );
    run(&mut world, rules::TOWER_RELEASE_COOLDOWN_TICKS);
    assert_eq!(
        shot_at_then_shed(&mut world),
        Some(own),
        "once the wait has run out, a click sheds it again"
    );
}
