//! Sylla's critical strike: its share of every block, what it is worth, and
//! how it is told of.

use crate::game::{
    AbilityBook, AbilityState, Entity, Event, MELEE_CREEP, UnitDef, World, ability, rules, wire_id,
};
use bota_proto::{EventKind, HeroId, SlotId, Team, Vec2};

use super::support::{ANVIL, assert_exact_blocks, opened, step_until, swinging_at, until_opened};

/// A melee attacker worth a hundred a strike on a short cycle, never
/// landing two strikes in one tick.
const STRIKER: UnitDef = UnitDef {
    attack_time: 300,
    attack_point: 100,
    attack_backswing: 100,
    damage: 100,
    ..MELEE_CREEP
};

/// `attacker` swinging as fast as a body may at an anvil in reach.
fn at_the_anvil(mut world: World, attacker: Entity) -> (World, Entity, Entity) {
    let anvil = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    world
        .modifiers
        .insert(attacker, swinging_at(rules::MAX_ATTACK_SPEED));
    world.settle();
    world.set_target(attacker, anvil);
    (world, attacker, anvil)
}

/// A striker with the crit at a level, set on an anvil.
fn striker_at_the_anvil(level: u8) -> (World, Entity, Entity) {
    let mut world = World::new();
    let striker = world.spawn_unit(&STRIKER, Team::Radiant, Vec2::from_ints(5000, 5000));
    world.abilities.insert(
        striker,
        AbilityBook {
            slots: vec![AbilityState {
                id: ability::CRIT,
                level,
                cooldown: 0,
            }],
            tree: None,
            talents: bota_proto::Talents::default(),
        },
    );
    at_the_anvil(world, striker)
}

/// Sylla with the crit at a level, set on an anvil on level ground.
fn sylla_at_the_anvil(level: u8) -> (World, Entity, Entity) {
    let mut world = World::new();
    let sylla = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        SlotId(0),
        HeroId(0),
    );
    world.abilities.get_mut(sylla).expect("has a book").slots[0].level = level;
    let (world, sylla, anvil) = at_the_anvil(world, sylla);
    let tier = |on: Entity| {
        world
            .ground
            .tier(world.transform.get(on).expect("standing").pos)
    };
    assert_eq!(
        tier(sylla),
        tier(anvil),
        "level ground, so nothing is missed uphill"
    );
    (world, sylla, anvil)
}

/// The blows one entity landed on another among a tick's events, in order,
/// as what each took off and whether it was a crit.
fn blows_in(events: &[Event], from: Entity, on: Entity) -> Vec<(i32, bool)> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Damaged {
                source,
                target,
                amount,
                crit,
                ..
            } if source == Some(wire_id(from)) && target == wire_id(on) => Some((amount, crit)),
            _ => None,
        })
        .collect()
}

/// The first `wanted` blows one entity lands on another.
fn blows_by(world: &mut World, from: Entity, on: Entity, wanted: usize) -> Vec<(i32, bool)> {
    let mut landed = Vec::new();
    step_until(world, &mut landed, wanted, |events| {
        blows_in(events, from, on)
    });
    landed
}

#[test]
fn every_full_block_of_swings_carries_exactly_the_level_share_of_crits() {
    for level in 1..=rules::ABILITY_MAX_LEVEL {
        let ratio = rules::SYLLA_CRIT_CHANCE[usize::from(level - 1)];
        let (mut world, striker, anvil) = striker_at_the_anvil(level);
        let told = |events: &[Event]| blows_in(events, striker, anvil);
        let (mut blows, first) =
            until_opened(&mut world, 20, told, |world| opened(&world.crit, striker));
        assert_eq!(blows.len(), 1, "the first swing landed the tick it rolled");
        step_until(
            &mut world,
            &mut blows,
            first + 5 * usize::from(ratio.den()),
            told,
        );
        assert_exact_blocks(
            &blows,
            first,
            ratio,
            |(_, crit)| *crit,
            &format!("level {level}"),
        );
    }
}

#[test]
fn a_crit_is_worth_its_level_multiple_of_a_plain_blow_and_says_so() {
    for level in 1..=rules::ABILITY_MAX_LEVEL {
        let pct = rules::SYLLA_CRIT_MULT_PCT[usize::from(level - 1)];
        let (mut world, sylla, anvil) = sylla_at_the_anvil(level);
        let blows = blows_by(&mut world, sylla, anvil, 40);
        assert_eq!(blows.len(), 40, "level {level}: forty shots landed");
        let plain = blows
            .iter()
            .find(|(_, crit)| !*crit)
            .map(|(amount, _)| *amount)
            .expect("some blow is plain");
        assert!(plain > 0, "a plain blow takes something off");
        assert!(
            blows.iter().any(|(_, crit)| *crit),
            "level {level}: forty shots carry a crit"
        );
        for (amount, crit) in blows {
            let wanted = if crit { plain * pct / 100 } else { plain };
            assert_eq!(amount, wanted, "level {level}: crit {crit}");
        }
    }
}

#[test]
fn without_the_crit_learned_nothing_is_rolled() {
    let (mut world, sylla, anvil) = sylla_at_the_anvil(0);
    let blows = blows_by(&mut world, sylla, anvil, 20);
    assert_eq!(blows.len(), 20, "twenty shots landed");
    assert!(blows.iter().all(|(_, crit)| !*crit), "no swing crits");
    assert!(
        opened(&world.crit, sylla).is_none(),
        "and no stream was opened"
    );
}
