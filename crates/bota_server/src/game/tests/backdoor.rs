//! Backdoor protection: a base building takes half the damage and mends it
//! while no enemy lane creep is near, and loses the protection for fifteen
//! and a half seconds whenever one is.

use bota_core::{map_of, rules};
use bota_proto::{DamageKind, EffectId, MapId, Team, Vec2};

use crate::game::{Entity, Hit, HitEffect, World};

use super::support::*;

/// Whether a side's view shows a structure wearing its protection, untimed
/// as Dota's buff is: both sides see it on every building.
fn shows(world: &World, side: Team, structure: Entity) -> bool {
    shown(world, Some(side), structure)
        .effects
        .iter()
        .any(|effect| effect.id == EffectId::BACKDOOR_PROTECTION && effect.ticks_left.is_none())
}

/// What one 400-damage blow of a Dire hero takes off the Radiant melee
/// barracks of the middle lane.
fn blow(world: &mut World, hero: crate::game::Entity, rax: crate::game::Entity) -> i32 {
    world.fill_pools(rax);
    world.hits.push_back(Hit {
        inflictor: None,
        source: Some(hero),
        target: rax,
        amount: 400,
        kind: DamageKind::Physical,
        damage_amp_bp: rules::NOMINAL_BP,
        attack: false,
        pierces: false,
        uphill: false,
        effect: HitEffect::None,
    });
    let before = world.health.get(rax).expect("standing").hp;
    world.step();
    (before - world.health.get(rax).expect("standing").hp).to_int()
}

#[test]
fn a_base_building_halves_and_mends_damage_until_an_enemy_creep_comes_near() {
    let mut world = World::on_map(map_of(MapId(0)));
    for tier in 0..3 {
        fell_at(&mut world, rules::RADIANT_TOWERS[tier].2);
    }
    let rax = structure_at(&world, rules::RADIANT_BARRACKS[0].2);
    let hero = seat_hero(&mut world, 0, Team::Dire, Vec2::from_ints(9000, 9000));
    world.settle();

    assert!(
        shows(&world, Team::Radiant, rax),
        "its own side sees it held"
    );
    assert!(shows(&world, Team::Dire, rax), "and so does the other");
    let held = blow(&mut world, hero, rax);
    let max = world.stats.get(rax).expect("settled").max_hp;
    assert!(held > 0);
    assert!(world.health.get(rax).expect("standing").hp < max);
    for _ in 0..rules::TICKS_PER_SECOND {
        world.step();
    }
    assert_eq!(
        world.health.get(rax).expect("standing").hp,
        max,
        "what enemy blows took is mended within a second"
    );

    let near = rules::RADIANT_ANCIENT_POS + Vec2::from_ints(600, 0);
    let creep = thinking_creep_of(&mut world, Team::Dire, near);
    world.step();
    let open = blow(&mut world, hero, rax);
    assert!((open - 2 * held).abs() <= 1, "{open} against {held}");
    assert!(
        !shows(&world, Team::Dire, rax),
        "an enemy creep near takes it off"
    );
    for _ in 0..rules::TICKS_PER_SECOND {
        world.step();
    }
    assert!(
        world.health.get(rax).expect("standing").hp < max,
        "unprotected damage stays"
    );

    world.bury(vec![(creep, None)], &mut Vec::new());
    for _ in 0..(15 * rules::TICKS_PER_SECOND) {
        world.step();
    }
    assert_eq!(
        blow(&mut world, hero, rax),
        open,
        "still open at fifteen seconds"
    );
    for _ in 0..(2 * rules::TICKS_PER_SECOND) {
        world.step();
    }
    assert_eq!(
        blow(&mut world, hero, rax),
        held,
        "held again after sixteen"
    );
    assert!(shows(&world, Team::Dire, rax), "and worn again");
}

#[test]
fn every_tower_past_the_first_tier_shows_it_and_a_tier_one_never_does() {
    let mut world = World::on_map(map_of(MapId(0)));
    world.step();
    for (tier, index) in [(1, 0), (2, 1), (3, 2)] {
        let tower = structure_at(&world, rules::RADIANT_TOWERS[index].2);
        for side in [Team::Radiant, Team::Dire] {
            assert_eq!(
                shows(&world, side, tower),
                tier >= 2,
                "tier {tier}, {side:?}"
            );
        }
    }
}
