//! The Glyph of Fortification (Dota 7.41f): one cooldown per side, seven
//! seconds for structures and three for lane creeps, five more targets for
//! a tower of tier two and above, a refresh when the first tier one tower,
//! tier two tower and melee barracks fall.

use bota_core::{MELEE_CREEP, ModifierKind, UnitDef, map_of, rules, tower_def};
use bota_proto::{
    DamageKind, EffectId, Fixed, GlyphView, MapId, Order, RejectReason, SlotId, Team, Vec2,
};

use crate::game::project::wire_id;
use crate::game::{Entity, Hit, HitEffect, Modifier, Place, World};

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
        inflictor: None,
        source: Some(from),
        target: on,
        amount: 100,
        kind: DamageKind::Pure,
        damage_amp_bp: rules::NOMINAL_BP,
        attack: false,
        striker: None,
        pierces: false,
        uphill: false,
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
    let mine = world.view(Team::Radiant).glyphs;
    assert_eq!(mine.len(), 2);
    assert!(mine[0].active_for > 0 && mine[0].ready_after > Some(0));
    assert_eq!((mine[1].team, mine[1].active_for), (Team::Dire, 0));
    let theirs = world.view(Team::Dire).glyphs;
    assert!(theirs[0].active_for > 0, "the enemy sees it is active");
    assert_eq!(
        theirs[0],
        GlyphView {
            team: Team::Radiant,
            ready_in: 0,
            active_for: mine[0].active_for,
            ready_after: None,
        },
        "and nothing of its cooldown, which is withheld"
    );

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

/// The tower's current target and the missiles in the air at one entity.
fn tower_at(world: &World, tower: Entity) -> Option<Entity> {
    world.target.get(tower).map(|t| t.0)
}

fn missiles_at(world: &World, tower: Entity, on: Entity) -> usize {
    world
        .entities
        .iter()
        .filter(|&e| {
            world
                .projectile
                .get(e)
                .is_some_and(|shot| shot.source == Some(tower) && shot.target == on)
        })
        .count()
}

#[test]
fn a_glyphed_creep_tanks_the_tower_for_a_diving_hero_until_the_hero_hits_a_hero() {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let tower = world.spawn_building(
        tower_def(1),
        Team::Dire,
        at,
        Place::Tower { lane: 1, tier: 1 },
    );
    let creep = world.spawn_unit(&TARGET, Team::Radiant, Vec2::from_ints(4500, 5000));
    let diver = seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(4400, 5000));
    let foe = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(4700, 5300));
    world.settle();
    world.fill_pools(creep);
    world.fill_pools(diver);
    // The creep is the first target in reach.
    world.put_modifier(
        creep,
        Modifier::new(ModifierKind::Fortified, None, None, Some(100_000)),
    );
    world.glyphs[0].structures_until = 100_000;
    let (creep_hp, diver_hp) = (
        world.health.get(creep).expect("up").hp,
        world.health.get(diver).expect("up").hp,
    );
    let mut shots = 0;
    for _ in 0..150 {
        world.step();
        shots += missiles_at(&world, tower, creep);
        assert_ne!(
            tower_at(&world, tower),
            Some(diver),
            "no aggro on the diver"
        );
    }
    assert!(shots > 0, "the tower keeps shooting the covered creep");
    assert_eq!(tower_at(&world, tower), Some(creep));
    assert_eq!(
        world.health.get(creep).expect("up").hp,
        creep_hp,
        "and it does no harm"
    );
    assert_eq!(world.health.get(diver).expect("up").hp, diver_hp);

    // The diver hits the enemy hero under the tower: Dota's aggro rule
    // draws the tower onto it as usual.
    attack_click(&mut world, foe);
    for _ in 0..30 {
        world.step();
    }
    assert_eq!(tower_at(&world, tower), Some(diver));
}

#[test]
fn the_enemy_sees_a_glyph_on_what_it_sees_and_nothing_on_what_the_fog_hides() {
    let mut world = World::on_map(map_of(MapId(0)));
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(3000, 3000));
    let at = rules::RADIANT_TOWERS[0].2;
    let near = |dx: i32| Vec2::from_ints(at.x.to_int() + dx, at.y.to_int());
    seat_hero(&mut world, 1, Team::Dire, near(1500));
    let tower = structure_at(&world, at);
    let hidden = world.spawn_unit(&TARGET, Team::Radiant, Vec2::from_ints(1000, 1000));
    let seen = world.spawn_unit(&TARGET, Team::Radiant, near(1450));
    world.settle();
    run_to(&mut world, rules::GLYPH_FIRST_USE_TICKS);
    // The buff's remaining ticks on a unit the Dire view holds: nothing for
    // a unit it is not told of.
    let buff = |world: &World, entity: Entity| {
        world
            .view(Team::Dire)
            .units
            .iter()
            .find(|u| u.id == wire_id(entity))
            .map(|u| {
                u.effects
                    .iter()
                    .find(|e| e.id == EffectId::FORTIFIED)
                    .and_then(|e| e.ticks_left)
            })
    };
    for entity in [tower, seen] {
        assert_eq!(buff(&world, entity), Some(None), "before the Glyph");
    }
    seat_order(&mut world, Order::Glyph);
    // The order took effect on the tick the step just ran.
    let used = world.tick - 1;
    assert_eq!(
        buff(&world, tower),
        Some(Some(used + rules::GLYPH_STRUCTURE_TICKS - world.tick)),
        "a building it sees, with its seven seconds counting down"
    );
    assert_eq!(
        buff(&world, seen),
        Some(Some(used + rules::GLYPH_CREEP_TICKS - world.tick)),
        "a creep it sees, with its three"
    );
    assert_eq!(
        buff(&world, hidden),
        None,
        "a creep in the fog is not there"
    );
    let after = used + rules::GLYPH_CREEP_TICKS + 2;
    run_to(&mut world, after);
    assert_eq!(
        buff(&world, seen),
        Some(None),
        "the creeps' three seconds are over"
    );
    assert_eq!(
        buff(&world, tower),
        Some(Some(used + rules::GLYPH_STRUCTURE_TICKS - world.tick)),
        "the building's are not"
    );
}

#[test]
fn a_creep_that_spawns_after_the_glyph_does_not_get_it() {
    let mut world = World::on_map(map_of(MapId(0)));
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(3000, 3000));
    let enemy = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(9000, 9000));
    let before = world.spawn_unit(&TARGET, Team::Radiant, Vec2::from_ints(3300, 3300));
    world.settle();
    run_to(&mut world, rules::GLYPH_FIRST_USE_TICKS);
    seat_order(&mut world, Order::Glyph);
    let later = world.tick + rules::TICKS_PER_SECOND;
    run_to(&mut world, later);
    let after = world.spawn_unit(&TARGET, Team::Radiant, Vec2::from_ints(3400, 3300));
    world.settle();
    assert!(
        world.tick < rules::GLYPH_FIRST_USE_TICKS + rules::GLYPH_CREEP_TICKS,
        "the wave comes inside the creeps' three seconds"
    );
    assert_eq!(
        pure_blow(&mut world, enemy, before),
        0,
        "the earlier wave is covered"
    );
    assert_eq!(
        pure_blow(&mut world, enemy, after),
        100,
        "the later one takes damage"
    );
}
