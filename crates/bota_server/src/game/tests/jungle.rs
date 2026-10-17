//! Neutral camps.

use bota_core::{CAMPS, MELEE_CREEP, OBSERVER_WARD, UnitDef, map_of, rules};

use crate::game::World;
use bota_proto::{DamageKind, MapId, Team, Vec2};

use super::support::*;

#[test]
fn camps_fill_on_the_minute_and_stay_full() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_NEUTRAL_TICK {
        world.step();
    }
    let neutrals = |world: &World| {
        world
            .entities
            .iter()
            .filter(|e| world.camp_home.get(*e).is_some())
            .count()
    };
    let filled = neutrals(&world);
    assert!(filled > 0, "the jungle put something out");
    for _ in 0..rules::NEUTRAL_SPAWN_PERIOD_TICKS {
        world.step();
    }
    assert_eq!(
        neutrals(&world),
        filled,
        "a full camp puts out nothing more"
    );
}

#[test]
fn a_neutral_led_too_far_gives_up_and_walks_home() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_NEUTRAL_TICK {
        world.step();
    }
    let beast = world
        .entities
        .iter()
        .find(|e| world.camp_home.get(*e).is_some())
        .expect("the jungle is full");
    let home = world.camp_home.get(beast).expect("it has one").home;
    // Carried well past its guard distance and held there.
    let away = home + Vec2::from_ints(1200, 0);
    if let Some(transform) = world.transform.get_mut(beast) {
        transform.pos = away;
    }
    for _ in 0..rules::NEUTRAL_AGGRO_WINDOW + 2 {
        world.step();
        if let Some(transform) = world.transform.get_mut(beast) {
            transform.pos = away;
        }
    }
    assert!(
        world.neutral_ai.get(beast).is_some_and(|ai| ai.going_home),
        "its patience ran out"
    );
    assert!(
        world.target.get(beast).is_none(),
        "and it takes nothing on while it walks back"
    );
    // Home again, it stands, and is led less far the next time.
    if let Some(transform) = world.transform.get_mut(beast) {
        transform.pos = home;
    }
    world.step();
    let ai = world.neutral_ai.get(beast).copied().expect("it has a mind");
    assert!(!ai.going_home, "back home it stands again");
    assert_eq!(
        ai.next_window,
        rules::NEUTRAL_SHORT_WINDOW,
        "and its patience is short from now on"
    );
}

#[test]
fn a_ward_in_a_camp_keeps_it_empty() {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    let camp = CAMPS[0].pos;
    world.spawn_unit(&OBSERVER_WARD, Team::Radiant, camp);
    world.settle();
    while world.tick < rules::FIRST_NEUTRAL_TICK + 1 {
        world.step();
    }
    let box_radius = rules::units(rules::CAMP_BOX_RADIUS);
    let filled = world.entities.iter().any(|entity| {
        world.team.get(entity) == Some(&Team::Neutral)
            && world
                .transform
                .get(entity)
                .is_some_and(|t| t.pos.within(camp, box_radius))
    });
    assert!(!filled, "a ward standing in the box keeps the camp empty");
}

#[test]
fn a_neutral_sleeps_until_something_comes_right_up_to_it() {
    let camp = Vec2::from_ints(5000, 5000);
    for (apart, wakes) in [
        (rules::NEUTRAL_AGGRO_RANGE + 120, false),
        (rules::NEUTRAL_AGGRO_RANGE - 60, true),
    ] {
        let mut world = World::new();
        world.tick = rules::PREGAME_TICKS;
        let beasts = a_camp_at(&mut world, camp);
        let hero = hero_at(&mut world, camp + Vec2::from_ints(apart, 0));
        world.settle();
        world.step();
        world.step();
        assert_eq!(
            world.target_of(beasts[0]) == Some(hero),
            wakes,
            "standing {apart} off by day, waking should be {wakes}"
        );
    }
}

#[test]
fn a_neutral_sleeps_through_the_night_with_an_enemy_at_its_side_and_wakes_at_dawn() {
    let camp = Vec2::from_ints(5000, 5000);
    let dawn = rules::PREGAME_TICKS + 2 * rules::HALF_DAY_TICKS;
    let mut world = World::new();
    world.tick = dawn - 40;
    let beasts = a_camp_at(&mut world, camp);
    // An onlooker that never swings: an idle hero swings at what stands in
    // its reach, and the blow would wake the camp by itself.
    const ONLOOKER: UnitDef = UnitDef {
        acquisition: 0,
        ..MELEE_CREEP
    };
    let onlooker = world.spawn_unit(
        &ONLOOKER,
        Team::Radiant,
        camp + Vec2::from_ints(rules::NEUTRAL_AGGRO_RANGE - 60, 0),
    );
    world.settle();
    while world.tick < dawn - 1 {
        world.step();
        assert!(
            beasts.iter().all(|beast| world.target_of(*beast).is_none()),
            "asleep at night on tick {}, whoever stands beside it",
            world.tick
        );
    }
    world.step();
    world.step();
    assert_eq!(
        world.target_of(beasts[0]),
        Some(onlooker),
        "the day wakes it to what stands beside it"
    );
}

#[test]
fn a_blow_wakes_a_camp_from_further_than_it_can_see() {
    let mut world = World::new();
    let camp = Vec2::from_ints(5000, 5000);
    let beasts = a_camp_at(&mut world, camp);
    // Far past anything they could see, but inside the reach of a blow.
    let apart = rules::NEUTRAL_DAMAGE_AGGRO_RANGE - 100;
    let hero = hero_at(&mut world, camp + Vec2::from_ints(apart, 0));
    world.settle();
    world.step();
    assert!(
        !world.can_see(Team::Neutral, hero),
        "they cannot see that far, which is the point"
    );
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "and they sleep through it"
    );
    world.push_hit(Some(hero), beasts[0], 10, DamageKind::Physical);
    world.step();
    world.step();
    for beast in &beasts {
        assert_eq!(
            world.target_of(*beast),
            Some(hero),
            "a blow wakes the whole camp, seen or not"
        );
    }
    // Further off than a blow carries, it wakes nobody.
    let mut world = World::new();
    let beasts = a_camp_at(&mut world, camp);
    let far = hero_at(
        &mut world,
        camp + Vec2::from_ints(rules::NEUTRAL_DAMAGE_AGGRO_RANGE + 400, 0),
    );
    world.settle();
    world.push_hit(Some(far), beasts[0], 10, DamageKind::Physical);
    world.step();
    world.step();
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "a blow from beyond its reach wakes nobody"
    );
}
