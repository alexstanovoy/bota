//! The buildings as the game's `npc_units.txt` (client 6957) has them:
//! health, regeneration, armor and sight, and the names a player reads.

use bota_core::{barracks_ranged_at, map_of, rules};
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
        if kind == UnitKind::Barracks {
            assert_eq!(
                barracks_ranged_at(world.map, team, pos),
                Some(hp == 1300),
                "{name}"
            );
        }
        seen += 1;
    }
    assert_eq!(seen, 2 + 12 + 14);
}

#[test]
fn a_barracks_is_ranged_by_where_it_stands_whatever_health_the_match_gives_it() {
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
    let mut ranged: Vec<Option<bool>> = view
        .units
        .iter()
        .filter(|unit| unit.kind == UnitKind::Barracks)
        .map(|unit| barracks_ranged_at(world.map, unit.team, unit.pos))
        .collect();
    ranged.sort_unstable();
    assert_eq!(
        ranged,
        [[Some(false); 6], [Some(true); 6]].concat(),
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

/// The health two units lose on each tick, over `ticks`.
fn losses(world: &mut World, units: [crate::game::Entity; 2], ticks: u32) -> Vec<[i32; 2]> {
    let hp = |world: &World, unit| world.health.get(unit).map_or(0, |h| h.hp.to_int());
    (0..ticks)
        .map(|_| {
            let before = units.map(|unit| hp(world, unit));
            world.step();
            [0, 1].map(|at| before[at] - hp(world, units[at]))
        })
        .collect()
}

/// Dota's Fountain Damage (`modifier_fountain_passive`; Liquipedia,
/// *Fountain*): each shot first stacks one more
/// `modifier_fountain_fury_swipes_damage_increase` on what it lands on, then
/// deals 3 more by every stack, the new one too; the stacks hold 12 s from
/// the last shot; a quarter of each shot splashes every other enemy within
/// 250; and a quarter of its shots go through evasion.
#[test]
fn the_fountains_shots_stack_three_more_each_hold_twelve_seconds_and_splash_a_quarter() {
    let mut world = World::on_map(map_of(MapId(2)));
    let fountain_at = world.map.fountains[0];
    let near = fountain_at + bota_proto::Vec2::from_ints(900, 0);
    let pair = [
        world.spawn_unit(&STANDING_TARGET, Team::Dire, near),
        world.spawn_unit(
            &STANDING_TARGET,
            Team::Dire,
            near + bota_proto::Vec2::from_ints(0, 200),
        ),
    ];
    world.settle();
    let fountain = super::support::structure_at(&world, fountain_at);
    assert_eq!(
        world.stats.get(fountain).map(|stats| stats.accuracy),
        Some(bota_core::Odds::percent(rules::FOUNTAIN_ACCURACY_PCT))
    );
    let seen = losses(&mut world, pair, 90);
    // The one shot at takes every full shot; the other its splash.
    let shot = usize::from(
        seen.iter().map(|lost| lost[1]).sum::<i32>() > seen.iter().map(|lost| lost[0]).sum::<i32>(),
    );
    let splashed = 1 - shot;
    let shots: Vec<i32> = seen
        .iter()
        .map(|lost| lost[shot])
        .filter(|&lost| lost > 0)
        .collect();
    let splashes: Vec<i32> = seen
        .iter()
        .map(|lost| lost[splashed])
        .filter(|&lost| lost > 0)
        .collect();
    assert!(shots.len() >= 10, "{shots:?}");
    let stacked: Vec<i32> = (1..=shots.len() as i32)
        .map(|stacks| rules::FOUNTAIN_ATTACK_DAMAGE + stacks * rules::FOUNTAIN_STACK_DAMAGE)
        .collect();
    assert_eq!(
        shots, stacked,
        "each shot 3 more than the last, the first too"
    );
    let quarters: Vec<i32> = stacked
        .iter()
        .map(|dealt| dealt * rules::FOUNTAIN_SPLASH_PCT / 100)
        .collect();
    assert_eq!(
        splashes,
        quarters[..splashes.len()],
        "a quarter splashes on the other"
    );
    assert!(splashes.len() + 1 >= shots.len(), "every shot splashes");
    let stacks_on = |world: &World, unit| {
        world
            .modifiers
            .get(unit)
            .map_or(0, |on_it| on_it.fountain_damage_stacks())
    };
    assert_eq!(stacks_on(&world, pair[shot]) as usize, shots.len());
    assert_eq!(
        stacks_on(&world, pair[splashed]),
        0,
        "a splash stacks nothing"
    );
    let view = world.view(Team::Dire);
    let worn = view
        .units
        .iter()
        .find(|unit| unit.id == crate::game::wire_id(pair[shot]))
        .and_then(|unit| {
            unit.effects
                .iter()
                .find(|e| e.id == bota_proto::EffectId::FOUNTAIN_DAMAGE)
        })
        .expect("the stacks are shown");
    assert_eq!(worn.stacks, Some(shots.len() as u32));
    assert_eq!(worn.ticks_total, Some(rules::FOUNTAIN_STACK_TICKS));
    // Carried out of reach, it keeps its stacks 12 s from the last shot.
    for unit in pair {
        if let Some(at) = world.transform.get_mut(unit) {
            at.pos = fountain_at + bota_proto::Vec2::from_ints(3000, 0);
        }
    }
    // The shots already in the air still land; the last of them renews them.
    let (mut last, mut since) = (stacks_on(&world, pair[shot]), 0);
    while stacks_on(&world, pair[shot]) > 0 {
        world.step();
        since += 1;
        let now = stacks_on(&world, pair[shot]);
        if now > last {
            (last, since) = (now, 0);
        }
        assert!(
            since <= rules::FOUNTAIN_STACK_TICKS,
            "the stacks outlast 12 s"
        );
    }
    assert_eq!(
        since,
        rules::FOUNTAIN_STACK_TICKS,
        "the stacks hold 12 s from the last shot"
    );
}

/// Dota's fountain reveals what hides within 1200 (Liquipedia, *Fountain*:
/// True Sight, "Reveal Radius: 1200"): an enemy observer ward planted in its
/// reach is seen by its side and shot down; one beyond is not seen.
#[test]
fn the_fountain_sees_a_hidden_ward_in_its_reach_and_shoots_it_down() {
    let mut world = World::on_map(map_of(MapId(2)));
    let fountain_at = world.map.fountains[0];
    let near = fountain_at + bota_proto::Vec2::from_ints(900, 0);
    let far = fountain_at + bota_proto::Vec2::from_ints(1500, 0);
    let plant = |world: &mut World, at| {
        world.spawn_ward(
            &bota_core::OBSERVER_WARD,
            (Team::Dire, at),
            10_000,
            (None, None),
        )
    };
    let (inside, outside) = (plant(&mut world, near), plant(&mut world, far));
    world.settle();
    assert!(world.can_see(Team::Radiant, inside), "true sight finds it");
    assert!(
        !world.can_see(Team::Radiant, outside),
        "beyond 1200 it hides"
    );
    for _ in 0..90 {
        world.step();
    }
    assert!(!world.alive(inside), "the fountain shoots it down");
    assert!(world.alive(outside));
}
