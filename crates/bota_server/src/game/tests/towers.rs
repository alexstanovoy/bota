//! Towers at Dota 7.41f: what they are made of, how fast they shoot, what
//! their fall pays and how often they let go.

use bota_core::{MELEE_CREEP, RANGED_CREEP, UnitDef, rules, tower_def};

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

/// Shots a tier-one tower needs to bring a fresh lane creep down.
fn shots_to_kill(def: &'static UnitDef, upgrades: u32) -> (i32, i32, u32) {
    let mut world = World::new();
    let tower = world.spawn_unit(tower_def(1), Team::Radiant, Vec2::from_ints(1500, 1500));
    // Out of the tower's own protection, which reaches only heroes anyway.
    let on = world.spawn_unit(def, Team::Dire, Vec2::from_ints(1900, 1500));
    world.set_upgrades(on, def, upgrades);
    world.settle();
    world.fill_pools(on);
    let stats = world.stats.get(on).expect("settled");
    let (hp, armor) = (stats.max_hp.to_int(), stats.armor.to_int());
    world.set_target(tower, on);
    let mut shots = 0;
    let mut before = world.health.get(on).expect("standing").hp;
    while world.alive(on) {
        world.step();
        assert!(world.tick < 3_000, "the tower shoots");
        let now = world.health.get(on).map_or(Fixed::ZERO, |health| health.hp);
        if now < before {
            shots += 1;
        }
        before = now;
    }
    (hp, armor, shots)
}

#[test]
fn a_tier_one_tower_needs_seven_or_eight_shots_for_a_melee_creep_and_four_for_a_ranged_one() {
    // npc_units.txt, 7.41f: the tower shoots 88 to 92 and is Reinforced,
    // which deals plain damage to anything but Reinforced; Liquipedia's
    // Lane Creeps: 550 health and armor 2 (melee), 300 and 0 (ranged), +12
    // health at 7:30 and no armor growth. Armor takes
    // 0.06 a / (1 + 0.06 a) off a shot. Never three: 270 is not 550.
    for (def, upgrades, hp, armor) in [
        (&MELEE_CREEP, 0, 550, 2),
        (&MELEE_CREEP, 1, 562, 2),
        (&RANGED_CREEP, 0, 300, 0),
        (&RANGED_CREEP, 1, 312, 0),
    ] {
        // The least a shot takes is 88 and the most 92, floored after armor;
        // the creep mends about three health in the seconds it takes.
        let through = |roll: i32| roll * 100 / (100 + 6 * armor);
        let least = (hp as u32).div_ceil(through(92) as u32);
        let most = (hp as u32 + 4).div_ceil(through(88) as u32);
        let (seen_hp, seen_armor, shots) = shots_to_kill(def, upgrades);
        assert_eq!((seen_hp, seen_armor), (hp, armor), "{:?}", def.kind);
        assert!(
            (least..=most).contains(&shots),
            "{:?} after {upgrades} upgrades fell to {shots} shots, Dota says {least} to {most}",
            def.kind
        );
    }
}
