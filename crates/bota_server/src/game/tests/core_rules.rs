//! The rule functions in `bota_core` that a client reads but the server never
//! calls, set against a world that plays by its own code.

use bota_core::{
    HEROES, aim_ticks, attributes_at_level, base_damage_least, facing_gap, facing_towards,
    heading_of, lane_creep_def, latest_wave, map_of, point_along, rules, tower_tier_at, wave_at,
    wave_upgrades,
};
use bota_proto::{Angle, Fixed, MapId, Team, UnitKind, Vec2};

use crate::game::{Level, UnitOrder, World};

use super::support::*;

#[test]
fn a_heros_view_shows_the_attributes_and_least_damage_the_core_computes_for_its_level() {
    for (index, hero_def) in HEROES.iter().enumerate() {
        let id = bota_proto::HeroId(index as u16);
        let mut world = World::new();
        let hero = seated_hero(&mut world, Vec2::from_ints(5000, 5000), id);
        for level in 1..=rules::HERO_MAX_LEVEL {
            world.level.insert(hero, Level(level));
            world.step();
            let shown = shown(&world, None, hero);
            assert_eq!(
                shown.attributes,
                attributes_at_level(hero_def.unit, level),
                "{} at level {level}",
                hero_def.name
            );
            assert_eq!(
                shown.attack_damage_min,
                base_damage_least(hero_def.unit, level),
                "{} at level {level}",
                hero_def.name
            );
        }
    }
}

#[test]
fn a_body_faces_its_target_in_the_ticks_aim_ticks_counts() {
    let at = Vec2::from_ints(5000, 5000);
    for brads in [0u16, 1_000, 5_000, 16_384, 32_768, 50_000, 65_000] {
        let mut world = World::new();
        let hero = hero_at(&mut world, at);
        let target = world.spawn_unit(
            &bota_core::MELEE_CREEP,
            Team::Dire,
            point_along(at, at + heading_of(Angle { brads }), Fixed::from_int(120)),
        );
        world.settle();
        let wanted = facing_towards(at, world.transform.get(target).expect("stands").pos);
        let facing = world.transform.get(hero).expect("stands").facing;
        let rate = world.stats.get(hero).expect("settled").turn_rate;
        let planned = aim_ticks(facing, wanted, rate);
        world.set_order(
            hero,
            UnitOrder::Attack {
                target,
                last_seen: world.transform.get(target).expect("stands").pos,
            },
        );
        let mut turned = 0;
        while facing_gap(world.transform.get(hero).expect("stands").facing, wanted)
            > rules::ATTACK_ANGLE_BRADS
        {
            world.step();
            turned += 1;
            assert!(turned <= 40, "it never turns, at {brads}");
        }
        assert_eq!(turned, planned, "{brads} brads");
    }
}

#[test]
fn the_tower_of_a_team_at_a_position_has_the_tier_the_core_names() {
    let map = map_of(MapId(2));
    let world = World::on_map(map);
    let mut towers = 0;
    for entity in world.entities.iter() {
        if world.kind.get(entity) != Some(&UnitKind::Tower) {
            continue;
        }
        let team = *world.team.get(entity).expect("has a side");
        let pos = world.transform.get(entity).expect("stands").pos;
        let tier = world.tier.get(entity).expect("has a tier").0;
        assert_eq!(tower_tier_at(map, team, pos), Some(tier));
        let other = if team == Team::Radiant {
            Team::Dire
        } else {
            Team::Radiant
        };
        assert_eq!(tower_tier_at(map, other, pos), None, "not the other side's");
        assert_eq!(
            tower_tier_at(map, team, pos + Vec2::from_ints(1, 0)),
            None,
            "only exactly at the tower"
        );
        towers += 1;
    }
    assert_eq!(towers, map.radiant_towers.len() + map.dire_towers.len());
}

#[test]
fn every_wave_has_the_number_upgrades_and_bodies_the_core_names() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let schedule = map.schedule();
    assert_eq!(latest_wave(0, schedule), None);
    let last = schedule.first_wave() + 15 * rules::WAVE_PERIOD_TICKS;
    let mut known = Vec::new();
    let mut waves = 0;
    let mut last_upgrades = 0;
    while world.tick < last + 2 {
        world.step();
        if wave_at(world.tick, schedule).is_some() {
            waves += 1;
        }
        assert_eq!(
            latest_wave(world.tick, schedule),
            (waves > 0).then_some(waves)
        );
        for creep in world
            .entities
            .iter()
            .filter(|e| world.march.get(*e).is_some())
        {
            if known.contains(&creep) {
                continue;
            }
            known.push(creep);
            let upgrades = world.upgrades.get(creep).map_or(0, |up| up.0);
            assert_eq!(upgrades, wave_upgrades(waves), "wave {waves}");
            last_upgrades = last_upgrades.max(upgrades);
            let kind = *world.kind.get(creep).expect("has a kind");
            let def = lane_creep_def(kind).expect("a lane creep");
            assert!(
                world.def.get(creep).expect("has a body").0 == def,
                "{kind:?} of wave {waves}"
            );
        }
    }
    assert_eq!(waves, 16);
    assert!(last_upgrades >= 1, "the sixteenth wave carries an upgrade");
}
