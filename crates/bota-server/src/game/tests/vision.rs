//! Who sees what, and what each side is told.

use crate::game::rules;
use crate::game::{Health, MELEE_CREEP, RANGED_CREEP, Visibility, World};
use bota_proto::{Fixed, Team};

use super::support::*;

#[test]
fn who_sees_an_entity_is_a_set_of_bits() {
    let mut seen = Visibility::default();
    assert!(!seen.by(Team::Radiant));
    seen.add(Team::Radiant);
    seen.add(Team::Radiant);
    assert!(seen.by(Team::Radiant), "naming a side twice sets one bit");
    assert!(!seen.by(Team::Dire), "and leaves the others alone");
    seen.add(Team::Dire);
    assert!(seen.by(Team::Radiant) && seen.by(Team::Dire));
    seen.clear();
    assert!(seen.is_empty() && !seen.by(Team::Dire));
}

#[test]
fn a_world_built_on_a_map_stands_its_buildings_full() {
    let map = crate::game::map_of(bota_proto::MapId(1));
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
        .find(|u| u.kind == bota_proto::UnitKind::Tower)
        .expect("a tower stands");
    assert_eq!(tower.hp, rules::TOWER_TIER_HP[0], "built and full");
    assert_eq!(tower.max_hp, rules::TOWER_TIER_HP[0]);
    // The demo map raises no Ancients at all.
    assert!(
        !view
            .units
            .iter()
            .any(|u| u.kind == bota_proto::UnitKind::Ancient),
        "no Ancient stands on the demo map"
    );
}

#[test]
fn both_sides_are_always_told_of_every_building() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let world = World::on_map(map);
    let standing = map_buildings(map);
    let view = world.view(bota_proto::Team::Radiant);
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
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let far = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Dire, map.fountains[1]);
    world.settle();
    assert!(
        !world.can_see(bota_proto::Team::Radiant, far),
        "a creep is not a building"
    );
    let view = world.view(bota_proto::Team::Radiant);
    assert!(
        !view.units.iter().any(|u| u.id == crate::game::wire_id(far)),
        "and the fog keeps it back"
    );
}

#[test]
fn a_unit_still_standing_never_reads_as_empty() {
    let mut world = World::new();
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::ZERO,
    );
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
    let watcher = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let near = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1200, 1000),
    );
    let far = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(9000, 9000),
    );
    world.settle();
    world.step();
    let seen_near = world.visibility.get(near).expect("worked out this tick");
    assert!(seen_near.by(bota_proto::Team::Radiant), "close enough");
    assert!(seen_near.by(bota_proto::Team::Dire), "its own side always");
    let seen_far = world.visibility.get(far).expect("worked out this tick");
    assert!(!seen_far.by(bota_proto::Team::Radiant), "out of sight");
    let view = world.view(bota_proto::Team::Radiant);
    let ids: Vec<_> = view.units.iter().map(|u| u.id).collect();
    assert!(ids.contains(&crate::game::wire_id(watcher)));
    assert!(ids.contains(&crate::game::wire_id(near)));
    assert!(
        !ids.contains(&crate::game::wire_id(far)),
        "fog holds it back"
    );
}

#[test]
fn a_missile_carries_the_hit_rather_than_landing_it_at_once() {
    let mut world = World::new();
    let archer = world.spawn_unit(
        &RANGED_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1400, 1000),
    );
    world.settle();
    let full = world.health.get(mark).expect("standing").hp;
    for _ in 0..20 {
        world.step();
    }
    assert!(
        world.projectile.get(archer).is_none(),
        "the archer is not its own missile"
    );
    let mut flying = 0;
    for entity in world.entities.iter() {
        if world.projectile.get(entity).is_some() {
            flying += 1;
        }
    }
    assert!(flying > 0 || world.health.get(mark).expect("standing").hp < full);
}

#[test]
fn a_side_is_told_only_of_what_it_could_see() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let far = bota_proto::Vec2::from_ints(9600, 12000);
    assert!(
        !world.can_see_point(bota_proto::Team::Radiant, far),
        "nothing of that side stands anywhere near"
    );
    assert_eq!(
        world.who_may_know(far, bota_proto::Team::Dire),
        crate::game::EventVisibility::OneTeam(bota_proto::Team::Dire),
        "only the side party to it is told"
    );
    let watcher = world.spawn_hero(
        bota_proto::Team::Radiant,
        far,
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    assert!(world.can_see_point(bota_proto::Team::Radiant, far));
    assert_eq!(
        world.who_may_know(far, bota_proto::Team::Dire),
        crate::game::EventVisibility::Everyone
    );
    let _ = watcher;
}

#[test]
fn critical_hits_keep_their_flag_in_damage_events() {
    let mut world = World::new();
    let source = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5_000, 5_000),
    );
    let target = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5_100, 5_000),
    );
    world.settle();
    world.hits.push_back(crate::game::Hit {
        source: Some(source),
        target,
        amount: 10,
        kind: bota_proto::DamageKind::Physical,
        damage_amp_bp: rules::NOMINAL_BP,
        crit: true,
        attack: true,
        pierces: false,
        effect: crate::game::HitEffect::None,
    });

    let events = world.step();

    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            bota_proto::EventKind::Damaged {
                source: Some(_),
                target: _,
                amount: _,
                kind: bota_proto::DamageKind::Physical,
                crit: true,
            }
        )
    }));
}

#[test]
fn a_ranged_attack_puts_a_missile_where_a_side_can_see_it() {
    let mut world = World::new();
    let archer = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    world.settle();
    world.fill_pools(archer);
    world.fill_pools(mark);
    let mut flew = false;
    for _ in 0..40 {
        world.step();
        if !world.view_full().projectiles.is_empty() {
            flew = true;
            break;
        }
    }
    assert!(
        flew,
        "a ranged hero throws something the client is told about"
    );
    let view = world.view(bota_proto::Team::Radiant);
    assert!(
        !view.projectiles.is_empty(),
        "and its own side is told of it"
    );
}

#[test]
fn a_visibility_row_belongs_to_whatever_stands_on_a_side() {
    let mut world = World::new();
    let entity = world.spawn();
    assert_eq!(
        world.visibility.get(entity),
        None,
        "what stands on no side is not something sides see"
    );
    world.set_team(entity, Team::Radiant);
    assert!(
        world
            .visibility
            .get(entity)
            .is_some_and(|s| s.by(Team::Radiant)),
        "taking a side makes the row, and that side has it at once"
    );
    world.step();
    assert!(
        world
            .visibility
            .get(entity)
            .is_some_and(|s| s.by(Team::Radiant)),
        "its own side has it from the first tick"
    );
    assert!(world.despawn(entity));
    assert_eq!(world.visibility.get(entity), None, "and given up with it");
    let next = world.spawn();
    assert_eq!(next.index(), entity.index(), "the slot came back round");
    assert_eq!(
        world.visibility.get(next),
        None,
        "and the new tenant inherits nothing"
    );
}

#[test]
fn a_missile_is_seen_from_the_tick_it_is_thrown() {
    let mut world = World::new();
    let archer = world.spawn_unit(
        &RANGED_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
    );
    let watcher = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    world.settle();
    let mut thrown = None;
    for _ in 0..40 {
        world.step();
        thrown = world
            .entities
            .iter()
            .find(|e| world.projectile.get(*e).is_some());
        if thrown.is_some() {
            break;
        }
    }
    let missile = thrown.expect("the archer threw something");
    assert!(
        world
            .visibility
            .get(missile)
            .is_some_and(|s| s.by(Team::Radiant)),
        "its own side has it"
    );
    let _ = (archer, watcher);
}
