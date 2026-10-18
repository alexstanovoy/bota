//! The Glyph of Fortification (Dota 7.41f): one cooldown per side, seven
//! seconds for structures and three for lane creeps, five more targets for
//! a tower of tier two and above, a refresh when the first tier one tower,
//! tier two tower and melee barracks fall.

use bota_core::{MELEE_CREEP, UnitDef, map_of, rules, tower_def};
use bota_proto::{DamageKind, Fixed, MapId, Order, RejectReason, SlotId, Team, Vec2};

use crate::game::{Entity, Hit, HitEffect, Place, World};

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

/// What a blow of 100 pure damage takes off an entity.
fn pure_blow(world: &mut World, from: Entity, on: Entity) -> i32 {
    world.fill_pools(on);
    world.hits.push_back(Hit {
        source: Some(from),
        target: on,
        amount: 100,
        kind: DamageKind::Pure,
        damage_amp_bp: rules::NOMINAL_BP,
        attack: false,
        pierces: false,
        effect: HitEffect::None,
    });
    let before = world.health.get(on).expect("standing").hp;
    world.step();
    (before - world.health.get(on).expect("standing").hp).to_int()
}

fn run_to(world: &mut World, tick: u32) {
    while world.tick < tick {
        world.step();
    }
}

#[test]
fn the_glyph_protects_structures_seven_seconds_and_creeps_three_then_recharges() {
    let mut world = World::on_map(map_of(MapId(0)));
    let hero = seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(3000, 3000));
    let enemy = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(9000, 9000));
    let tower = structure_at(&world, rules::RADIANT_TOWERS[0].2);
    let creep = thinking_creep_of(&mut world, Team::Radiant, Vec2::from_ints(3300, 3300));
    world.settle();

    let order = Order::Glyph;
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::OnCooldown),
        "not before three minutes past the horn"
    );
    run_to(&mut world, rules::GLYPH_FIRST_USE_TICKS);
    assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
    assert_eq!(pure_blow(&mut world, enemy, tower), 100);
    assert_eq!(pure_blow(&mut world, hero, creep), 100);

    seat_order(&mut world, order);
    let used = world.tick;
    assert_eq!(
        pure_blow(&mut world, enemy, tower),
        0,
        "structures take nothing"
    );
    assert_eq!(pure_blow(&mut world, enemy, creep), 0, "nor do lane creeps");
    let dire = world.view(Team::Dire).glyphs;
    assert!(
        dire.iter()
            .all(|g| g.team == Team::Dire && g.active_for == 0)
    );
    let mine = world.view(Team::Radiant).glyphs;
    assert_eq!(mine.len(), 1);
    assert!(mine[0].active_for > 0 && mine[0].ready_in > 0);

    run_to(&mut world, used + rules::GLYPH_CREEP_TICKS + 2);
    assert_eq!(
        pure_blow(&mut world, enemy, creep),
        100,
        "creeps after three seconds"
    );
    assert_eq!(pure_blow(&mut world, enemy, tower), 0, "structures still");
    run_to(&mut world, used + rules::GLYPH_STRUCTURE_TICKS + 2);
    assert_eq!(
        pure_blow(&mut world, enemy, tower),
        100,
        "structures after seven"
    );

    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::OnCooldown)
    );
    assert_eq!(
        world.validate_order(SlotId(1), None, &order),
        Ok(()),
        "the other side has a Glyph of its own"
    );
    run_to(&mut world, used + rules::GLYPH_COOLDOWN_TICKS);
    assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
}

#[test]
fn the_first_tier_one_tower_tier_two_tower_and_melee_barracks_to_fall_refresh_it() {
    let mut world = World::on_map(map_of(MapId(0)));
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(3000, 3000));
    run_to(&mut world, rules::GLYPH_FIRST_USE_TICKS);
    let order = Order::Glyph;
    let fall = |world: &mut World, at: Vec2| {
        let it = structure_at(world, at);
        world.bury(vec![(it, None)], &mut Vec::new());
    };
    let spend = |world: &mut World| {
        seat_order(world, order);
        assert_eq!(
            world.validate_order(SlotId(0), None, &order),
            Err(RejectReason::OnCooldown)
        );
    };
    let steps = [
        rules::RADIANT_TOWERS[0].2,
        rules::RADIANT_TOWERS[1].2,
        rules::RADIANT_BARRACKS[0].2,
    ];
    for (n, at) in steps.into_iter().enumerate() {
        spend(&mut world);
        fall(&mut world, at);
        let after = world.tick + rules::GLYPH_REFRESH_TICKS + 1;
        run_to(&mut world, after);
        assert_eq!(
            world.validate_order(SlotId(0), None, &order),
            Ok(()),
            "fall {n}"
        );
    }
    // A second tier one tower, and every later one, refreshes nothing.
    spend(&mut world);
    fall(&mut world, rules::RADIANT_TOWERS[3].2);
    let after = world.tick + rules::GLYPH_REFRESH_TICKS + 1;
    run_to(&mut world, after);
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::OnCooldown)
    );
}

/// How many missiles the tower has in the air the tick after its first
/// shot is thrown.
fn missiles_of_a_volley(tier: u8, glyphed: bool, enemies: usize) -> usize {
    let mut world = World::new();
    let at = Vec2::from_ints(1500, 1500);
    let tower = world.spawn_building(
        tower_def(tier),
        Team::Radiant,
        at,
        Place::Tower { lane: 1, tier },
    );
    for n in 0..enemies {
        let spot = Vec2::from_ints(1900 + 20 * n as i32, 1500 + 40 * n as i32);
        world.spawn_unit(&TARGET, Team::Dire, spot);
    }
    world.settle();
    if glyphed {
        world.glyphs[0].structures_until = 100_000;
    }
    for _ in 0..600 {
        world.step();
        let flying = world
            .entities
            .iter()
            .filter(|&e| {
                world
                    .projectile
                    .get(e)
                    .is_some_and(|shot| shot.source == Some(tower))
            })
            .count();
        if flying > 0 {
            return flying;
        }
    }
    panic!("the tower never shot");
}

#[test]
fn a_glyphed_tower_of_tier_two_and_above_shoots_five_more_targets_a_tier_one_does_not() {
    assert_eq!(missiles_of_a_volley(2, false, 8), 1);
    assert_eq!(missiles_of_a_volley(2, true, 8), 6);
    assert_eq!(missiles_of_a_volley(2, true, 3), 3, "as many as there are");
    assert_eq!(missiles_of_a_volley(4, true, 8), 6);
    assert_eq!(missiles_of_a_volley(1, true, 8), 1);
}
