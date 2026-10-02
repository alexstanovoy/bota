//! Fountain, tower and flagbearer auras.

use crate::game::rules;
use crate::game::{FLAGBEARER_CREEP, Health, MELEE_CREEP, World};
use bota_proto::{Fixed, Team};

use super::support::*;

#[test]
fn a_fountain_hands_out_mending_to_whoever_stands_in_it() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    world.health.insert(
        hero,
        Health {
            hp: Fixed::from_int(100),
        },
    );
    world.step();
    assert!(
        carries(&world, hero, FOUNTAIN_EFFECT),
        "standing in it, the hero carries the effect"
    );
    assert!(
        world.health.get(hero).map(|h| h.hp)
            >= Some(Fixed::from_int(100 + rules::FOUNTAIN_HEAL_HP_PER_TICK)),
        "and mends by at least what the fountain hands out"
    );
    // Walked out of reach, it runs out on its own: nothing takes it off, and
    // what it has left is what the fountain last handed it.
    world.transform.get_mut(hero).expect("hero").pos = bota_proto::Vec2::from_ints(9600, 9216);
    world.step();
    assert!(
        carries(&world, hero, FOUNTAIN_EFFECT),
        "one step out it is still running"
    );
    for _ in 0..rules::TICKS_PER_SECOND {
        world.step();
    }
    assert!(
        !carries(&world, hero, FOUNTAIN_EFFECT),
        "a second later it is gone"
    );
}

#[test]
fn a_tower_guards_the_heroes_of_its_own_side_that_stand_by_it() {
    // Every tier, since what a tower adds is not the same at each of them.
    for tier in 1..=4u8 {
        let mut world = World::new();
        let at = bota_proto::Vec2::from_ints(5000, 5000);
        world.spawn_unit(crate::game::tower_def(tier), Team::Radiant, at);
        let hero = world.spawn_hero(
            Team::Radiant,
            bota_proto::Vec2::from_ints(5400, 5000),
            bota_proto::SlotId(0),
            bota_proto::HeroId(0),
        );
        let bare = world.spawn_hero(
            Team::Radiant,
            bota_proto::Vec2::from_ints(5000 + rules::TOWER_AURA_RADIUS + 200, 5000),
            bota_proto::SlotId(1),
            bota_proto::HeroId(0),
        );
        world.settle();
        world.step();
        assert!(
            carries(&world, hero, GUARDED_EFFECT),
            "tier {tier}: inside the reach"
        );
        assert!(
            !carries(&world, bare, GUARDED_EFFECT),
            "tier {tier}: outside it"
        );
        let added = rules::TOWER_AURA_ARMOR[usize::from(tier) - 1];
        assert_eq!(
            world.stats.get(hero).map(|s| s.armor),
            world
                .stats
                .get(bare)
                .map(|s| s.armor + Fixed::from_int(added)),
            "tier {tier}: the armor it hands out is on the stat"
        );
    }
}

#[test]
fn a_tower_guards_nobody_but_heroes() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    world.spawn_unit(crate::game::tower_def(1), Team::Radiant, at);
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5400, 5000),
    );
    let theirs = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5100),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    assert!(
        !carries(&world, creep, GUARDED_EFFECT),
        "a wave under its own tower is not what the protection is for"
    );
    assert!(
        !carries(&world, theirs, GUARDED_EFFECT),
        "and it reaches its own side only"
    );
}

#[test]
fn a_flagbearer_inspires_everyone_of_its_own_side_around_it() {
    let mut world = World::new();
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    world.spawn_unit(&FLAGBEARER_CREEP, Team::Radiant, at);
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    let far = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5000 + rules::FLAGBEARER_AURA_RADIUS + 200, 5000),
    );
    let hero = world.spawn_hero(
        Team::Radiant,
        bota_proto::Vec2::from_ints(5300, 5100),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let theirs = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5300, 4900),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    assert!(carries(&world, creep, INSPIRED_EFFECT), "inside the reach");
    assert!(!carries(&world, far, INSPIRED_EFFECT), "outside it");
    assert!(
        carries(&world, hero, INSPIRED_EFFECT),
        "it inspires its own heroes as readily as its own creeps"
    );
    assert!(
        !carries(&world, theirs, INSPIRED_EFFECT),
        "and its own side only"
    );
}

#[test]
fn a_fountain_hands_out_nothing_to_the_other_side() {
    let mut world = World::for_match(&config(), config().rng());
    let hero = world.seats[0].unit.expect("stood up");
    let theirs = world.spawn_hero(
        bota_proto::Team::Dire,
        world.transform.get(hero).expect("standing").pos,
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.step();
    assert!(
        !carries(&world, theirs, FOUNTAIN_EFFECT),
        "an enemy standing in it mends no faster for it"
    );
}

#[test]
fn the_fountain_melts_whoever_steps_into_its_reach_and_spares_who_stays_out() {
    let cfg = crate::game::MatchConfig {
        match_id: 13,
        master_key: [0; 32],
        picks: vec![bota_proto::Pick {
            slot: bota_proto::SlotId(0),
            team: bota_proto::Team::Dire,
            hero: bota_proto::HeroId(1),
        }],
        map: bota_proto::MapId(0),
        tick_rate: 30,
        mode: bota_proto::TickMode::Realtime,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("stood up");
    world.level.insert(hero, crate::game::Level(10));
    world.settle();
    world.fill_pools(hero);
    let fountain = rules::RADIANT_FOUNTAIN_POS;
    let hold = |world: &mut World, hero, at, ticks| {
        for _ in 0..ticks {
            world.advance(&[]);
            if let Some(t) = world.transform.get_mut(hero) {
                t.pos = at;
            }
        }
    };
    // Past its reach, nothing comes out.
    let outside = fountain + bota_proto::Vec2::from_ints(1400, 0);
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = outside;
    }
    let full = world.health.get(hero).expect("standing").hp;
    hold(&mut world, hero, outside, 45);
    assert_eq!(
        world.health.get(hero).expect("standing").hp,
        full,
        "out of reach the fountain leaves it be"
    );
    // Inside, a hero of the tenth level is torn apart within seconds.
    let inside = fountain + bota_proto::Vec2::from_ints(1000, 0);
    if let Some(t) = world.transform.get_mut(hero) {
        t.pos = inside;
    }
    // The first shots are still in the air: the reading starts once the
    // missiles have begun to land.
    hold(&mut world, hero, inside, 30);
    let before = world.health.get(hero).expect("standing").hp.to_int();
    hold(&mut world, hero, inside, 30);
    let after = world.health.get(hero).map_or(0, |h| h.hp.to_int());
    assert!(
        before - after > 1200,
        "a second under the fountain costs over 1200 health, not {}",
        before - after
    );
}
