//! Sylla's critical strike: its share of every block, what it is worth, and
//! how it is told of.

use bota_proto::{EventKind, Fixed, HeroId, SlotId, Team, Vec2};

use crate::game::{
    AbilityBook, AbilityState, Entity, Event, MELEE_CREEP, Modifier, ModifierKind, Modifiers,
    UnitDef, World, ability, rules, wire_id,
};

/// What takes a strike and never falls: no armor, no mending, no swing back.
const ANVIL: UnitDef = UnitDef {
    max_hp: 30_000,
    armor: 0,
    damage: 0,
    hp_regen: Fixed::ZERO,
    ..MELEE_CREEP
};

/// A melee attacker worth a hundred a strike on a short cycle, never
/// landing two strikes in one tick.
const STRIKER: UnitDef = UnitDef {
    attack_time: 300,
    attack_point: 100,
    attack_backswing: 100,
    damage: 100,
    ..MELEE_CREEP
};

/// Swinging as fast as a body may.
fn fastest() -> Modifiers {
    Modifiers(vec![Modifier {
        kind: ModifierKind::Haste {
            speed: rules::MAX_ATTACK_SPEED - rules::BASE_ATTACK_SPEED,
        },
        source: None,
        ticks_left: Some(u32::MAX / 2),
    }])
}

/// A striker with the crit at a level, set on an anvil in reach.
fn striker_at_the_anvil(level: u8) -> (World, Entity, Entity) {
    let mut world = World::new();
    let striker = world.spawn_unit(&STRIKER, Team::Radiant, Vec2::from_ints(5000, 5000));
    let anvil = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    world.abilities.insert(
        striker,
        AbilityBook {
            slots: vec![AbilityState {
                id: ability::CRIT,
                level,
                cooldown: 0,
            }],
        },
    );
    world.modifiers.insert(striker, fastest());
    world.settle();
    world.set_target(striker, anvil);
    (world, striker, anvil)
}

/// Sylla with the crit at a level, set on an anvil in reach on level ground.
fn sylla_at_the_anvil(level: u8) -> (World, Entity, Entity) {
    let mut world = World::new();
    let sylla = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        SlotId(0),
        HeroId(0),
    );
    let anvil = world.spawn_unit(&ANVIL, Team::Dire, Vec2::from_ints(5100, 5000));
    if let Some(book) = world.abilities.get_mut(sylla) {
        book.slots[0].level = level;
    }
    world.modifiers.insert(sylla, fastest());
    world.settle();
    world.set_target(sylla, anvil);
    let (Some(from), Some(to)) = (
        world.transform.get(sylla).map(|t| t.pos),
        world.transform.get(anvil).map(|t| t.pos),
    ) else {
        panic!("both stand");
    };
    assert_eq!(
        world.ground.tier(from),
        world.ground.tier(to),
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

/// Steps until so many blows have landed, adding them to what was seen.
fn land_blows(
    world: &mut World,
    from: Entity,
    on: Entity,
    seen: &mut Vec<(i32, bool)>,
    wanted: usize,
) {
    for _ in 0..800 {
        if seen.len() >= wanted {
            return;
        }
        seen.extend(blows_in(&world.step(), from, on));
    }
}

/// Blows one entity landed on another, in order. Stops once so many have
/// landed.
fn blows_by(world: &mut World, from: Entity, on: Entity, wanted: usize) -> Vec<(i32, bool)> {
    let mut landed = Vec::new();
    land_blows(world, from, on, &mut landed, wanted);
    landed
}

/// The stream of crit rolls an attacker has opened, if it has.
fn crit_stream(world: &World, attacker: Entity) -> Option<&crate::game::Chance> {
    world
        .crit
        .get(attacker.index().0 as usize)
        .and_then(|chance| chance.as_ref())
}

/// How many rolls of the first block the first roll left, itself included.
///
/// A stream opens mid-block; read right after its first roll, its position
/// says where it opened.
fn first_block_len(world: &World, attacker: Entity, den: usize) -> usize {
    let chance = crit_stream(world, attacker).expect("the first swing opened the stream");
    match usize::from(chance.block_position()) {
        0 => 1,
        pos => den - (pos - 1),
    }
}

#[test]
fn every_full_block_of_swings_carries_exactly_the_level_share_of_crits() {
    for level in 1..=rules::ABILITY_MAX_LEVEL {
        let ratio = rules::SYLLA_CRIT_CHANCE[usize::from(level - 1)];
        let (num, den) = (usize::from(ratio.num()), usize::from(ratio.den()));
        let (mut world, striker, anvil) = striker_at_the_anvil(level);
        // Step to the first swing, which opens the stream.
        let mut blows = Vec::new();
        let mut first = None;
        for _ in 0..20 {
            blows.extend(blows_in(&world.step(), striker, anvil));
            if crit_stream(&world, striker).is_some() {
                first = Some(first_block_len(&world, striker, den));
                break;
            }
        }
        let first = first.expect("a swing came within twenty ticks");
        assert_eq!(blows.len(), 1, "the first swing landed the tick it rolled");
        land_blows(&mut world, striker, anvil, &mut blows, first + den * 5);
        let full = blows[first..].chunks_exact(den);
        assert!(full.len() >= 5, "level {level}: five full blocks landed");
        for (nth, block) in full.enumerate() {
            let crits = block.iter().filter(|(_, crit)| *crit).count();
            assert_eq!(crits, num, "level {level}, block {nth}: {block:?}");
        }
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
        crit_stream(&world, sylla).is_none(),
        "and no stream was opened"
    );
}
