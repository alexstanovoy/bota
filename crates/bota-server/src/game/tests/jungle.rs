//! Neutral camps.

use crate::game::World;
use crate::game::rules;

use super::support::*;

#[test]
fn camps_fill_on_the_minute_and_stay_full() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_NEUTRAL_TICK {
        world.step();
    }
    let filled = world
        .entities
        .iter()
        .filter(|e| world.camp_home.get(*e).is_some())
        .count();
    assert!(filled > 0, "the jungle put something out");
    let before = filled;
    for _ in 0..rules::NEUTRAL_SPAWN_PERIOD_TICKS {
        world.step();
    }
    let now = world
        .entities
        .iter()
        .filter(|e| world.camp_home.get(*e).is_some())
        .count();
    assert_eq!(now, before, "a full camp puts out nothing more");
}

#[test]
fn a_neutral_led_too_far_gives_up_and_walks_home() {
    let map = crate::game::map_of(bota_proto::MapId(1));
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
    let away = home + bota_proto::Vec2::from_ints(1200, 0);
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
    let map = crate::game::map_of(bota_proto::MapId(0));
    let mut world = World::on_map(map);
    let camp = crate::game::CAMPS[0].pos;
    world.spawn_unit(&crate::game::OBSERVER_WARD, bota_proto::Team::Radiant, camp);
    world.settle();
    while world.tick < rules::FIRST_NEUTRAL_TICK + 1 {
        world.step();
    }
    let box_radius = rules::units(rules::CAMP_BOX_RADIUS);
    let filled = world.entities.iter().any(|entity| {
        world.team.get(entity) == Some(&bota_proto::Team::Neutral)
            && world
                .transform
                .get(entity)
                .is_some_and(|t| t.pos.within(camp, box_radius))
    });
    assert!(!filled, "a ward standing in the box keeps the camp empty");
}

#[test]
fn a_neutral_sleeps_until_something_comes_right_up_to_it() {
    let camp = bota_proto::Vec2::from_ints(5000, 5000);
    for (apart, wakes) in [
        (rules::NEUTRAL_AGGRO_RANGE + 120, false),
        (rules::NEUTRAL_AGGRO_RANGE - 60, true),
    ] {
        let mut world = World::new();
        let beasts = a_camp_at(&mut world, camp);
        let hero = world.spawn_hero(
            bota_proto::Team::Radiant,
            camp + bota_proto::Vec2::from_ints(apart, 0),
            bota_proto::SlotId(0),
            bota_proto::HeroId(0),
        );
        world.settle();
        world.step();
        world.step();
        assert_eq!(
            world.target_of(beasts[0]) == Some(hero),
            wakes,
            "standing {apart} off, waking should be {wakes}"
        );
    }
}

#[test]
fn a_blow_wakes_a_camp_from_further_than_it_can_see() {
    let mut world = World::new();
    let camp = bota_proto::Vec2::from_ints(5000, 5000);
    let beasts = a_camp_at(&mut world, camp);
    // Far past anything they could see, but inside the reach of a blow.
    let apart = rules::NEUTRAL_DAMAGE_AGGRO_RANGE - 100;
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        camp + bota_proto::Vec2::from_ints(apart, 0),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    assert!(
        !world.can_see(bota_proto::Team::Neutral, hero),
        "they cannot see that far, which is the point"
    );
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "and they sleep through it"
    );
    world.push_hit(Some(hero), beasts[0], 10, bota_proto::DamageKind::Physical);
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
    let far = world.spawn_hero(
        bota_proto::Team::Radiant,
        camp + bota_proto::Vec2::from_ints(rules::NEUTRAL_DAMAGE_AGGRO_RANGE + 400, 0),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.push_hit(Some(far), beasts[0], 10, bota_proto::DamageKind::Physical);
    world.step();
    world.step();
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "a blow from beyond its reach wakes nobody"
    );
}
