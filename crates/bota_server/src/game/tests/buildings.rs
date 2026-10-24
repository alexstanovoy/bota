//! The buildings as the game's `npc_units.txt` (client 6957) has them:
//! health, regeneration, armor and sight, and the names a player reads.

use bota_core::{map_of, rules, unit_name};
use bota_proto::{Fixed, MapId, ModifierSpec, Team, UnitKind};

use crate::game::{ModifierDuration, SpawnModifier, SpawnSelector, SpawnTarget, World};

#[test]
fn every_building_stands_as_the_unit_table_has_it() {
    let world = World::on_map(map_of(MapId(0)));
    let mut seen = 0;
    for entity in world.entities.iter() {
        let (Some(kind), Some(team)) = (
            world.kind.get(entity).copied(),
            world.team.get(entity).copied(),
        ) else {
            continue;
        };
        let stats = world.stats.get(entity).expect("settled");
        let hp = stats.max_hp.to_int();
        let regen = (stats.hp_regen.to_f32() * rules::TICKS_PER_SECOND as f32).round() as i32;
        let (armor, per_second, name) = match (kind, hp) {
            (UnitKind::Ancient, 4500) => (
                23,
                12,
                if team == Team::Radiant {
                    "Radiant's Ancient"
                } else {
                    "Dire's Ancient"
                },
            ),
            (UnitKind::Barracks, 2200) => (15, 5, "Melee Barracks"),
            (UnitKind::Barracks, 1300) => (9, 0, "Ranged Barracks"),
            (UnitKind::Filler, 1000) => (10, 0, "Building"),
            (UnitKind::Tower, _) | (UnitKind::Fountain, _) => continue,
            other => panic!("a building the table does not know: {other:?}"),
        };
        assert_eq!(stats.armor, Fixed::from_int(armor), "{name}");
        assert_eq!(regen, per_second, "{name}");
        let pos = world.transform.get(entity).expect("standing").pos;
        assert_eq!(unit_name(world.map, kind, team, pos), name);
        seen += 1;
    }
    assert_eq!(seen, 2 + 12 + 14);
}

#[test]
fn a_barracks_is_named_by_where_it_stands_whatever_health_the_match_gives_it() {
    let mut world = World::on_map(map_of(MapId(0)));
    world.spawn_modifiers = vec![SpawnModifier {
        select: SpawnSelector {
            team: None,
            targets: vec![SpawnTarget::Kind(UnitKind::Barracks)],
        },
        spec: ModifierSpec {
            max_hp: 17_000,
            ..ModifierSpec::NOMINAL
        },
        duration: ModifierDuration::MatchLong,
    }];
    world.apply_spawn_modifiers_to_all();
    world.settle();
    let view = world.view(Team::Dire);
    let mut names: Vec<&str> = view
        .units
        .iter()
        .filter(|unit| unit.kind == UnitKind::Barracks)
        .map(|unit| unit_name(world.map, unit.kind, unit.team, unit.pos))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [["Melee Barracks"; 6], ["Ranged Barracks"; 6]].concat(),
        "both sides' barracks, read off a view"
    );
}

/// A creep that bears no armor and never falls to a few shots.
static STANDING_TARGET: bota_core::UnitDef = bota_core::UnitDef {
    max_hp: 30_000,
    armor: 0,
    hp_regen: Fixed::ZERO,
    move_speed: 0,
    ..bota_core::MELEE_CREEP
};

/// The ticks, from now, on which a unit loses health over `ticks`.
fn blows_on(world: &mut World, on: crate::game::Entity, ticks: u32) -> Vec<u32> {
    let mut blows = Vec::new();
    for tick in 0..ticks {
        let before = world.health.get(on).map(|h| h.hp);
        world.step();
        if world.health.get(on).map(|h| h.hp) < before {
            blows.push(tick);
        }
    }
    blows
}

/// Dota's fountain (`dota_fountain`, 7.41f): `MovementTurnRate` 0, so it
/// shoots on every side at once, 6.67 shots a second (`AttackRate` 0.15, no
/// attack point), 1200 far. On the Dota map it faced one way and shot only
/// what stood in front of it, as if it had to turn to shoot.
#[test]
fn the_fountain_shoots_every_enemy_in_its_reach_on_every_side_six_and_two_thirds_times_a_second() {
    // The Radiant fountain sits in the map's corner: across the quarter its
    // base opens to, from the river side round to the jungle side.
    let sides = [(1000, 0), (900, 450), (700, 700), (450, 900), (0, 1000)];
    for (side, (x, y)) in sides.into_iter().enumerate() {
        let mut world = World::on_map(map_of(MapId(2)));
        let at = world.map.fountains[0] + bota_proto::Vec2::from_ints(x, y);
        let creep = world.spawn_unit(&STANDING_TARGET, Team::Dire, at);
        world.settle();
        let blows = blows_on(&mut world, creep, 90);
        let first = *blows
            .first()
            .unwrap_or_else(|| panic!("side {side}: never shot"));
        assert!(first < 30, "side {side}: first blow on tick {first}");
        // Every 4.5 ticks at 30 a second: 13 or 14 blows in two seconds.
        let landed = blows
            .iter()
            .filter(|&&t| (first..first + 60).contains(&t))
            .count();
        assert!(
            (13..=14).contains(&landed),
            "side {side}: {landed} blows in two seconds"
        );
    }
    let mut world = World::on_map(map_of(MapId(2)));
    let fountain = world.map.fountains[1];
    let hero = super::support::seat_hero(
        &mut world,
        0,
        Team::Radiant,
        fountain + bota_proto::Vec2::from_ints(-700, -700),
    );
    world.settle();
    world.fill_pools(hero);
    assert!(
        !blows_on(&mut world, hero, 30).is_empty(),
        "an enemy hero in the Dire fountain's reach is shot"
    );
}
