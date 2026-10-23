//! Evasion: the share of attacks at a carrier that miss, held exactly and
//! told of; what is not an attack never misses.

use bota_core::{MELEE_CREEP, RANGED_CREEP, Ratio, UnitDef, rules};

use crate::game::{Entity, Event, World, wire_id};
use bota_proto::{DamageKind, EventKind, Fixed, HeroId, ItemId, SlotId, Team, Vec2};

use super::support::{assert_exact_blocks, hold, opened, step_until, swing_at, until_opened};

/// A melee attacker worth a point a strike on a short cycle, never landing
/// two strikes in one tick, never falling and never bringing a hero down.
const STRIKER: UnitDef = UnitDef {
    max_hp: 30_000,
    attack_time: 300,
    attack_point: 100,
    attack_backswing: 100,
    damage: 1,
    ..MELEE_CREEP
};

/// One that never swings at all.
const BYSTANDER: UnitDef = UnitDef {
    damage: 0,
    ..STRIKER
};

/// The same, throwing its strikes.
const ARCHER: UnitDef = UnitDef {
    max_hp: 30_000,
    attack_time: 300,
    attack_point: 100,
    attack_backswing: 100,
    damage: 1,
    projectile_speed: Some(6_000),
    ..RANGED_CREEP
};

/// An attacker of a kind set on a hero in reach, the hero holding what it
/// is given.
fn attacker_at_a_hero(attacker: &'static UnitDef, held: &[ItemId]) -> (World, Entity, Entity) {
    let mut world = World::new();
    let from = world.spawn_unit(attacker, Team::Radiant, Vec2::from_ints(5000, 5000));
    let mark = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5100, 5000),
        SlotId(0),
        HeroId::PUDGE,
    );
    for (slot, item) in held.iter().enumerate() {
        hold(&mut world, mark, slot, *item);
    }
    swing_at(&mut world, from, rules::MAX_ATTACK_SPEED);
    world.settle();
    world.fill_pools(mark);
    world.set_target(from, mark);
    (world, from, mark)
}

/// What one entity's attacks at another came to among a tick's events, in
/// order: true for a blow that landed, false for one that missed.
fn swings_in(events: &[Event], from: Entity, on: Entity) -> Vec<bool> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Damaged { source, target, .. }
                if source == Some(wire_id(from)) && target == wire_id(on) =>
            {
                Some(true)
            }
            EventKind::Missed { source, target }
                if source == Some(wire_id(from)) && target == wire_id(on) =>
            {
                Some(false)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_butterfly_makes_exactly_its_share_of_melee_and_thrown_swings_miss() {
    let ratio = Ratio::new(7, 20);
    for (attacker, case) in [(&STRIKER, "melee"), (&ARCHER, "thrown")] {
        let (mut world, from, mark) = attacker_at_a_hero(attacker, &[ItemId::BUTTERFLY]);
        assert_eq!(
            world.stats.get(mark).map(|stats| stats.evasion),
            Some(ratio),
            "the Butterfly is worth its evasion"
        );
        let told = |events: &[Event]| swings_in(events, from, mark);
        let (mut swings, first) =
            until_opened(&mut world, 40, told, |world| opened(&world.evasion, mark));
        assert_eq!(swings.len(), 1, "{case}: told of the tick it rolled");
        step_until(
            &mut world,
            &mut swings,
            first + 5 * usize::from(ratio.den()),
            told,
        );
        assert_exact_blocks(&swings, first, ratio, |landed| !*landed, case);
    }
}

#[test]
fn what_is_not_an_attack_never_misses() {
    let (mut world, from, mark) = attacker_at_a_hero(&BYSTANDER, &[ItemId::BUTTERFLY]);
    let mut blows = Vec::new();
    for _ in 0..40 {
        world.push_hit(Some(from), mark, 10, DamageKind::Magical, None);
        blows.extend(swings_in(&world.step(), from, mark));
    }
    assert_eq!(blows.len(), 40, "every blow of the spell was told of");
    assert!(
        blows.iter().all(|landed| *landed),
        "and none of them missed"
    );
}

#[test]
fn without_evasion_nothing_is_rolled_and_nothing_misses() {
    let (mut world, from, mark) = attacker_at_a_hero(&STRIKER, &[]);
    let mut swings = Vec::new();
    step_until(&mut world, &mut swings, 30, |events| {
        swings_in(events, from, mark)
    });
    assert_eq!(swings.len(), 30, "thirty swings were told of");
    assert!(swings.iter().all(|landed| *landed), "all of them landed");
    assert!(
        opened(&world.evasion, mark).is_none(),
        "and no stream was opened"
    );
}

#[test]
fn of_two_evasions_carried_the_better_counts() {
    let (world, _from, mark) =
        attacker_at_a_hero(&STRIKER, &[ItemId::TALISMAN_OF_EVASION, ItemId::BUTTERFLY]);
    assert_eq!(
        world.stats.get(mark).map(|stats| stats.evasion),
        Some(Ratio::new(7, 20))
    );
    let (world, _from, mark) = attacker_at_a_hero(&STRIKER, &[ItemId::TALISMAN_OF_EVASION]);
    assert_eq!(
        world.stats.get(mark).map(|stats| stats.evasion),
        Some(Ratio::new(3, 20))
    );
}

#[test]
fn a_butterfly_is_worth_its_agility_damage_and_share_of_the_base_pace() {
    let (world, _from, plain) = attacker_at_a_hero(&STRIKER, &[]);
    let bare = *world.stats.get(plain).expect("settled");
    let (world, _from, mark) = attacker_at_a_hero(&STRIKER, &[ItemId::BUTTERFLY]);
    let with = *world.stats.get(mark).expect("settled");
    assert_eq!(
        with.attributes.agility - bare.attributes.agility,
        Fixed::from_int(30)
    );
    // Thirty of its own; Pudge's primary is strength, so agility adds none.
    assert_eq!(with.damage - bare.damage, 30);
    let agility_pace =
        (with.attributes.agility * Fixed::from_int(rules::ATTACK_SPEED_PER_AGILITY)).to_int();
    let base = rules::BASE_ATTACK_SPEED + agility_pace;
    assert_eq!(
        with.attack_speed,
        base + base * 20 / 100,
        "a fifth of the base pace and of what agility adds, and nothing else"
    );
}
