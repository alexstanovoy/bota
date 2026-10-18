//! Dota 7.41f's attack classes: what share of a swing reaches each kind of
//! target, swung for real.

use bota_core::{
    HERO, MEGA_MELEE_CREEP, MELEE_CREEP, NeutralKind, RANGED_CREEP, SHADOW_FIEND, SIEGE_CREEP,
    SUPER_MELEE_CREEP, TOWERS, UnitDef,
};

use crate::game::World;
use bota_proto::{Attributes, Team, UnitKind, Vec2};

use super::support::*;

/// A target of a kind that bears no armor and never falls to one swing.
const fn dummy(kind: UnitKind) -> UnitDef {
    UnitDef {
        kind,
        attributes: Attributes::ZERO,
        primary: None,
        max_hp: 9_000,
        damage: 0,
        armor: 0,
        move_speed: 0,
        ..MELEE_CREEP
    }
}

const HERO_DUMMY: UnitDef = dummy(UnitKind::Hero);
const CREEP_DUMMY: UnitDef = dummy(UnitKind::CreepMelee);
const SIEGE_DUMMY: UnitDef = dummy(UnitKind::CreepSiege);
const TOWER_DUMMY: UnitDef = dummy(UnitKind::Tower);
const NEUTRAL_DUMMY: UnitDef = dummy(UnitKind::CreepNeutral);

/// An attacker of a kind whose every swing is worth the same.
const fn steady(def: &UnitDef) -> UnitDef {
    UnitDef {
        damage_spread: 0,
        ..*def
    }
}

const MELEE: UnitDef = steady(&MELEE_CREEP);
const RANGED: UnitDef = steady(&RANGED_CREEP);
const SIEGE: UnitDef = steady(&SIEGE_CREEP);
const KOBOLD: UnitDef = steady(NeutralKind::Kobold.def());
const TOWER: UnitDef = steady(&TOWERS[0]);

/// What one swing of `attacker` takes off a dummy of each kind, against
/// what the swing is worth.
fn swing_at(attacker: &'static UnitDef, target: &'static UnitDef) -> (i32, i32) {
    let mut world = World::new();
    let from = world.spawn_unit(attacker, Team::Radiant, Vec2::from_ints(5000, 5000));
    let on = world.spawn_unit(target, Team::Dire, Vec2::from_ints(5110, 5000));
    world.settle();
    world.fill_pools(on);
    let worth = world.stats.get(from).expect("settled").damage;
    (one_swing_takes(&mut world, from, on), worth)
}

#[test]
fn every_attack_class_lands_its_share_on_every_kind() {
    // Percent of a swing that lands on: hero, creep, siege creep, tower,
    // neutral. Liquipedia, Attack Damage: Runty 0.75 to heroes and 0.7 to
    // Reinforced; Piercing 0.5, 1.5 and 0.35; Reinforced 2.5 to Reinforced;
    // a hero 0.5 to Reinforced; the jungle and the rest have no class.
    let table: [(&'static UnitDef, [i32; 5]); 6] = [
        (&MELEE, [75, 100, 70, 70, 100]),
        (&RANGED, [50, 150, 35, 35, 150]),
        (&SIEGE, [100, 100, 250, 250, 100]),
        (&TOWER, [100, 100, 250, 250, 100]),
        (&HERO, [100, 100, 50, 50, 100]),
        (&KOBOLD, [100, 100, 100, 100, 100]),
    ];
    let targets = [
        &HERO_DUMMY,
        &CREEP_DUMMY,
        &SIEGE_DUMMY,
        &TOWER_DUMMY,
        &NEUTRAL_DUMMY,
    ];
    for (attacker, shares) in table {
        for (target, pct) in targets.into_iter().zip(shares) {
            let (taken, worth) = swing_at(attacker, target);
            assert_eq!(
                taken,
                worth * pct / 100,
                "{:?} at {:?}: {pct} % of {worth}",
                attacker.kind,
                target.kind
            );
        }
    }
}

#[test]
fn every_swing_draws_its_damage_evenly_from_the_range() {
    // Liquipedia, Attack Damage: "every time an attack is issued, a random
    // value between the minimum and maximum is chosen". A melee creep swings
    // 19 to 23; Shadow Fiend 16 to 22 base, and his 25 agility on top.
    for (attacker, low, high) in [(&MELEE_CREEP, 19, 23), (&SHADOW_FIEND, 41, 47)] {
        // In a corner of the plain map, where no wave ever walks by.
        let mut world = World::new();
        let from = world.spawn_unit(attacker, Team::Radiant, Vec2::from_ints(1500, 1500));
        let on = world.spawn_unit(&CREEP_DUMMY, Team::Dire, Vec2::from_ints(1610, 1500));
        world.settle();
        let swings = 300;
        let taken: Vec<i32> = (0..swings)
            .map(|_| {
                world.fill_pools(on);
                one_swing_takes(&mut world, from, on)
            })
            .collect();
        let label = format!("{:?}", attacker.kind);
        assert_eq!(taken.iter().min(), Some(&low), "{label}: the least");
        assert_eq!(taken.iter().max(), Some(&high), "{label}: the most");
        let mean_x10 = taken.iter().sum::<i32>() * 10 / swings;
        assert!(
            (mean_x10 - (low + high) * 5).abs() <= 3,
            "{label}: mean {mean_x10} tenths"
        );
    }
}

#[test]
fn the_view_tells_the_range_every_swing_really_draws_from() {
    // A hero, a lane creep, a super one, a mega one (a fixed hit) and a tower.
    let attackers: [&'static UnitDef; 5] = [
        &SHADOW_FIEND,
        &MELEE_CREEP,
        &SUPER_MELEE_CREEP,
        &MEGA_MELEE_CREEP,
        &TOWERS[0],
    ];
    for attacker in attackers {
        let mut world = World::new();
        let from = world.spawn_unit(attacker, Team::Radiant, Vec2::from_ints(1500, 1500));
        let on = world.spawn_unit(&CREEP_DUMMY, Team::Dire, Vec2::from_ints(1610, 1500));
        world.settle();
        let unit = world
            .view_full()
            .units
            .into_iter()
            .find(|unit| unit.id == crate::game::wire_id(from))
            .expect("the attacker is in the view");
        let taken: Vec<i32> = (0..200)
            .map(|_| {
                world.fill_pools(on);
                one_swing_takes(&mut world, from, on)
            })
            .collect();
        let label = format!("{:?}", attacker.kind);
        assert_eq!(
            taken.iter().min(),
            Some(&unit.attack_damage_min),
            "{label}: the least"
        );
        assert_eq!(
            taken.iter().max(),
            Some(&unit.attack_damage_max),
            "{label}: the most"
        );
        assert_eq!(
            unit.attack_damage,
            (unit.attack_damage_min + unit.attack_damage_max) / 2,
            "{label}: the middle"
        );
    }
}
