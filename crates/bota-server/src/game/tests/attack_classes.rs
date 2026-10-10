//! Dota 7.41f's attack classes: what share of a swing reaches each kind of
//! target, swung for real.

use crate::game::{
    HERO, MELEE_CREEP, NeutralKind, RANGED_CREEP, SIEGE_CREEP, UnitDef, World, tower_def,
};
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
    let tower = tower_def(1);
    let kobold = NeutralKind::Kobold.def();
    // Percent of a swing that lands on: hero, creep, siege creep, tower,
    // neutral. Liquipedia, Attack Damage: Runty 0.75 to heroes and 0.7 to
    // Reinforced; Piercing 0.5, 1.5 and 0.35; Reinforced 2.5 to Reinforced;
    // a hero 0.5 to Reinforced; the jungle and the rest have no class.
    let table: [(&'static UnitDef, [i32; 5]); 6] = [
        (&MELEE_CREEP, [75, 100, 70, 70, 100]),
        (&RANGED_CREEP, [50, 150, 35, 35, 150]),
        (&SIEGE_CREEP, [100, 100, 250, 250, 100]),
        (tower, [100, 100, 250, 250, 100]),
        (&HERO, [100, 100, 50, 50, 100]),
        (kobold, [100, 100, 100, 100, 100]),
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
