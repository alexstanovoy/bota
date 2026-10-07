//! Who sees what, and what each side is told.

use crate::game::{
    EventVisibility, Health, MELEE_CREEP, RANGED_CREEP, World, map_of, rules, wire_id,
};
use bota_proto::{Fixed, MapId, Team, UnitKind, Vec2};

use super::support::*;

#[test]
fn a_world_built_on_a_map_stands_its_buildings_full() {
    let map = map_of(MapId(1));
    let world = World::on_map(map);
    assert_eq!(
        world.entities.len(),
        map_buildings(map),
        "every building and nothing else"
    );
    let view = world.view_full();
    assert_eq!(view.units.len(), world.entities.len());
    let tower = view
        .units
        .iter()
        .find(|u| u.kind == UnitKind::Tower)
        .expect("a tower stands");
    assert_eq!(tower.hp, rules::TOWER_TIER_HP[0], "built and full");
    assert_eq!(tower.max_hp, rules::TOWER_TIER_HP[0]);
    // The demo map raises no Ancients at all.
    assert!(
        !view.units.iter().any(|u| u.kind == UnitKind::Ancient),
        "no Ancient stands on the demo map"
    );
}

#[test]
fn both_sides_are_always_told_of_every_building() {
    let map = map_of(MapId(1));
    let world = World::on_map(map);
    let standing = map_buildings(map);
    let view = world.view(Team::Radiant);
    assert_eq!(
        view.units.len(),
        standing,
        "every building on the map is told, whosever it is"
    );
    let dire_fountain = map.fountains[1];
    assert!(
        view.units.iter().any(|u| u.pos == dire_fountain),
        "including the one across the map it has no eyes on"
    );
}

#[test]
fn a_unit_across_the_map_is_still_kept_from_a_side() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let far = world.spawn_unit(&MELEE_CREEP, Team::Dire, map.fountains[1]);
    world.settle();
    assert!(
        !world.can_see(Team::Radiant, far),
        "a creep is not a building"
    );
    let view = world.view(Team::Radiant);
    assert!(
        !view.units.iter().any(|u| u.id == wire_id(far)),
        "and the fog keeps it back"
    );
}

#[test]
fn a_unit_still_standing_never_reads_as_empty() {
    let mut world = World::new();
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::ZERO);
    world.settle();
    world.health.insert(
        creep,
        Health {
            hp: Fixed::from_ratio(1, 4),
        },
    );
    let view = world.view_full();
    let shown = view.units.first().expect("one unit").hp;
    assert_eq!(shown, 1, "a quarter of a point still shows as one");
}

#[test]
fn a_side_sees_what_stands_inside_its_sight() {
    let mut world = World::new();
    let watcher = hero_at(&mut world, Vec2::from_ints(1000, 1000));
    let near = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(1200, 1000));
    let far = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(9000, 9000));
    world.settle();
    world.step();
    let seen_near = world.visibility.get(near).expect("worked out this tick");
    assert!(seen_near.by(Team::Radiant), "close enough");
    assert!(seen_near.by(Team::Dire), "its own side always");
    let seen_far = world.visibility.get(far).expect("worked out this tick");
    assert!(!seen_far.by(Team::Radiant), "out of sight");
    let view = world.view(Team::Radiant);
    let ids: Vec<_> = view.units.iter().map(|u| u.id).collect();
    assert!(ids.contains(&wire_id(watcher)));
    assert!(ids.contains(&wire_id(near)));
    assert!(!ids.contains(&wire_id(far)), "fog holds it back");
}

#[test]
fn a_side_is_told_only_of_what_it_could_see() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let far = Vec2::from_ints(9600, 12000);
    assert!(
        !world.can_see_point(Team::Radiant, far),
        "nothing of that side stands anywhere near"
    );
    assert_eq!(
        world.who_may_know(far, Team::Dire),
        EventVisibility::OneTeam(Team::Dire),
        "only the side party to it is told"
    );
    hero_at(&mut world, far);
    world.settle();
    assert!(world.can_see_point(Team::Radiant, far));
    assert_eq!(
        world.who_may_know(far, Team::Dire),
        EventVisibility::Everyone
    );
}

#[test]
fn a_ranged_attack_puts_a_missile_where_a_side_can_see_it() {
    let mut world = World::new();
    let archer = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5300, 5000));
    world.settle();
    world.fill_pools(archer);
    world.fill_pools(mark);
    let flew = (0..40).any(|_| {
        world.step();
        !world.view_full().projectiles.is_empty()
    });
    assert!(
        flew,
        "a ranged hero throws something the client is told about"
    );
    let view = world.view(Team::Radiant);
    assert!(
        !view.projectiles.is_empty(),
        "and its own side is told of it"
    );
}

#[test]
fn a_missile_is_seen_from_the_tick_it_is_thrown() {
    let mut world = World::new();
    world.spawn_unit(&RANGED_CREEP, Team::Radiant, Vec2::from_ints(5000, 5000));
    world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5300, 5000));
    world.settle();
    let missile = (0..40)
        .find_map(|_| {
            world.step();
            world
                .entities
                .iter()
                .find(|e| world.projectile.get(*e).is_some())
        })
        .expect("the archer threw something");
    assert!(
        world
            .visibility
            .get(missile)
            .is_some_and(|s| s.by(Team::Radiant)),
        "its own side has it"
    );
}
