//! Roshan through the match and order paths.
use super::support::*;
use crate::game::World;
use bota_proto::{MapId, ObjectiveKind, ObjectiveSet};

#[test]
fn roshan_stands_at_match_start_only_when_the_map_and_objectives_allow_him() {
    for (map, enabled, expected) in [(0, true, 1), (2, true, 1), (1, true, 0), (0, false, 0)] {
        let mut cfg = config();
        cfg.map = MapId(map);
        cfg.objectives = if enabled {
            ObjectiveSet::of(ObjectiveKind::Roshan)
        } else {
            ObjectiveSet::NONE
        };
        if map == 1 {
            assert!(cfg.validate().is_err());
            continue;
        }
        let world = World::for_match(&cfg, cfg.rng());
        let count = world
            .entities
            .iter()
            .filter(|unit| {
                world
                    .neutral
                    .get(*unit)
                    .is_some_and(|kind| kind.neutral().unit == "npc_dota_roshan")
            })
            .count();
        assert_eq!(count, expected, "map {map}, enabled {enabled}");
    }
}

fn match_with_roshan() -> World {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = ObjectiveSet::of(ObjectiveKind::Roshan);
    let mut world = World::for_match(&cfg, cfg.rng());
    world.creeps_halted = true;
    world
}

fn boss(world: &World) -> crate::game::Entity {
    world.lairs[0].unit.expect("Roshan standing")
}

#[test]
fn roshan_waits_until_fifteen_minutes_and_walks_between_pits_on_natural_transitions() {
    use bota_core::{ROSHAN_DAY_PIT, ROSHAN_NIGHT_PIT, rules};
    let mut world = match_with_roshan();
    let it = boss(&world);
    assert_eq!(world.pos(it), Some(ROSHAN_DAY_PIT));
    world.clock_shift.game = world.schedule().horn + 7 * 60 * rules::TICKS_PER_SECOND;
    world.step();
    assert_eq!(world.camp_home.get(it).expect("home").home, ROSHAN_DAY_PIT);
    world.clock_shift.game =
        world.schedule().horn + 15 * 60 * rules::TICKS_PER_SECOND - world.tick - 1;
    world.step();
    assert_eq!(
        world.camp_home.get(it).expect("new home").home,
        ROSHAN_NIGHT_PIT
    );
    assert!(
        world
            .pos(it)
            .expect("walking")
            .within(ROSHAN_DAY_PIT, rules::units(100)),
        "does not teleport"
    );
    for _ in 0..90 * rules::TICKS_PER_SECOND {
        world.step();
    }
    assert!(
        world
            .pos(it)
            .expect("arrived")
            .within(ROSHAN_NIGHT_PIT, rules::units(100))
    );
    world.clock_shift.game =
        world.schedule().horn + 20 * 60 * rules::TICKS_PER_SECOND - world.tick - 1;
    world.step();
    assert_eq!(
        world.camp_home.get(it).expect("day home").home,
        ROSHAN_DAY_PIT
    );
}

#[test]
fn roshan_respawns_within_eight_to_eleven_minutes_in_the_current_pit() {
    use bota_core::{ROSHAN_NIGHT_PIT, rules};
    use bota_proto::DamageKind;
    let mut waits = Vec::new();
    for stream in 0..4 {
        let mut world = match_with_roshan();
        world.rng = crate::game::MatchRng::new(&[3; 32], stream);
        let it = boss(&world);
        let hero = world.seats[0].unit.expect("hero");
        world.push_hit(Some(hero), it, 100_000, DamageKind::Pure, None);
        world.step();
        assert!(world.lairs[0].unit.is_none());
        let at = world.lairs[0].respawn_at;
        let delay = at - world.clock();
        assert!(
            (8 * 60 * rules::TICKS_PER_SECOND..=11 * 60 * rules::TICKS_PER_SECOND).contains(&delay)
        );
        waits.push(delay);
        world.clock_shift.game = at - world.tick - 2;
        world.step();
        assert!(world.lairs[0].unit.is_none());
        world.step();
        assert!(world.lairs[0].unit.is_some());
        // A later death at 15:00 returns during the night of 25:00-30:00.
        world.clock_shift.game =
            world.schedule().horn + 16 * 60 * rules::TICKS_PER_SECOND - world.tick;
        let it = boss(&world);
        world.push_hit(Some(hero), it, 100_000, DamageKind::Pure, None);
        world.step();
        let at = world.lairs[0].respawn_at;
        world.clock_shift.game = at - world.tick - 1;
        world.step();
        let expected = if bota_core::time_of_day(world.clock(), world.schedule()).night {
            ROSHAN_NIGHT_PIT
        } else {
            bota_core::ROSHAN_DAY_PIT
        };
        assert_eq!(world.pos(boss(&world)), Some(expected));
    }
    assert!(waits.windows(2).any(|w| w[0] != w[1]));
}

#[test]
fn roshan_can_be_attacked_from_inside_either_pit_and_ignores_outside_attacks() {
    use crate::game::{Command, wire_id};
    use bota_core::rules;
    use bota_proto::{Fixed, Order, Target, Team, Vec2};
    for inside in [false, true] {
        let mut world = match_with_roshan();
        let it = boss(&world);
        let at = world.pos(it).expect("boss");
        let hero = world.seats[0].unit.expect("hero");
        // A ranged hero has reach in both cases; a ward grants sight inside.
        let spot = at + Vec2::from_ints(if inside { 200 } else { -450 }, 0);
        assert_eq!(
            world.map.boss_zones.iter().any(|zone| zone.contains(spot)),
            inside
        );
        world.transform.get_mut(hero).expect("hero position").pos = spot;
        world.spawn_unit(
            &bota_core::OBSERVER_WARD,
            Team::Radiant,
            at + Vec2::from_ints(0, 50),
        );
        world.settle();
        let order = Order::Attack {
            target: Target::Unit(wire_id(it)),
        };
        assert_eq!(
            world.validate_order(world.seats[0].slot, None, &order),
            Ok(())
        );
        world.advance(&[Command {
            slot: world.seats[0].slot,
            unit: None,
            order,
            seq: 0,
            queue: false,
        }]);
        let mut blows = 0;
        for _ in 0..4 * rules::TICKS_PER_SECOND {
            blows += world.step().iter().filter(|event| matches!(event.kind(),bota_proto::EventKind::Damaged { target, inflictor:None, .. } if target==wire_id(it))).count();
        }
        assert_eq!(blows > 0, inside);
        assert!(world.health.get(it).expect("alive").hp > Fixed::ZERO);
    }
}

#[test]
fn flying_vision_cannot_cross_a_pit_boundary_in_either_direction() {
    use bota_core::{RANGED_CREEP, ROSHAN_DAY_PIT, UnitDef, rules};
    use bota_proto::{Team, Vec2};
    const FLYER: UnitDef = UnitDef {
        flies: true,
        day_vision: 2000,
        night_vision: 2000,
        ..RANGED_CREEP
    };
    for (from, to) in [
        (ROSHAN_DAY_PIT + Vec2::from_ints(-700, 0), ROSHAN_DAY_PIT),
        (ROSHAN_DAY_PIT, ROSHAN_DAY_PIT + Vec2::from_ints(-700, 0)),
    ] {
        let mut world = match_with_roshan();
        let eye = world.spawn_unit(&FLYER, Team::Radiant, from);
        let mark = world.spawn_unit(&RANGED_CREEP, Team::Dire, to);
        world.settle();
        assert!(
            !world.can_see(Team::Radiant, mark),
            "flying viewer {eye:?} across the boundary"
        );
        world.transform.get_mut(eye).expect("eye").pos = to + Vec2::from_ints(0, 50);
        world.settle();
        assert!(
            world.can_see(Team::Radiant, mark),
            "within the same zone, in range {}",
            rules::TICKS_PER_SECOND
        );
    }
}

#[test]
fn hurting_a_camp_creep_does_not_wake_an_independent_boss() {
    use bota_core::{NeutralKind, ROSHAN_DAY_PIT};
    use bota_proto::{DamageKind, Vec2};
    let mut world = match_with_roshan();
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero position").pos =
        ROSHAN_DAY_PIT + Vec2::from_ints(1000, 0);
    let creep = world.spawn_neutral(
        NeutralKind::Kobold,
        ROSHAN_DAY_PIT + Vec2::from_ints(200, 0),
        0,
        0,
    );
    world.settle();
    world.push_hit(Some(hero), creep, 1, DamageKind::Pure, None);
    world.step();
    assert!(!world.neutral_ai.get(boss(&world)).expect("boss mind").awake);
    assert!(world.neutral_ai.get(creep).expect("camp mind").awake);
}

#[test]
fn a_blow_does_not_interrupt_roshans_walk_to_the_other_pit() {
    use bota_core::{ROSHAN_DAY_PIT, rules};
    use bota_proto::{DamageKind, Vec2};
    let mut world = match_with_roshan();
    let it = boss(&world);
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero position").pos =
        ROSHAN_DAY_PIT + Vec2::from_ints(500, 0);
    world.clock_shift.game = world.schedule().horn + 15 * 60 * rules::TICKS_PER_SECOND - 1;
    world.step();
    world.push_hit(Some(hero), it, 1, DamageKind::Pure, None);
    world.step();
    let ai = world.neutral_ai.get(it).expect("neutral mind");
    assert!(ai.going_home);
    assert!(!ai.awake);
}

#[test]
fn roshan_wakes_at_night_and_returns_home_after_being_led_beyond_his_leash() {
    use bota_core::{ROSHAN_DAY_PIT, rules};
    use bota_proto::Vec2;
    let mut world = match_with_roshan();
    world.immortal_heroes = true;
    let it = boss(&world);
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero position").pos =
        ROSHAN_DAY_PIT + Vec2::from_ints(90, 0);
    world.clock_shift.game = world.schedule().horn + 7 * 60 * rules::TICKS_PER_SECOND;
    world.settle();
    world.step();
    assert!(
        world.neutral_ai.get(it).expect("mind").awake,
        "does not sleep by night"
    );
    world.step();
    assert_eq!(world.target_of(it), Some(hero));
    world.transform.get_mut(hero).expect("hero position").pos = world.map.fountains[0];
    world.transform.get_mut(it).expect("boss position").pos =
        ROSHAN_DAY_PIT + Vec2::from_ints(1000, 0);
    world.target.remove(it);
    world.set_order(it, crate::game::UnitOrder::Idle);
    world.settle();
    run(&mut world, bota_core::ROSHAN_HOME.window + 1);
    assert!(world.neutral_ai.get(it).expect("leashed").going_home);
    run(&mut world, 15 * rules::TICKS_PER_SECOND);
    assert!(
        world
            .pos(it)
            .expect("home")
            .within(ROSHAN_DAY_PIT, rules::units(100))
    );
}
