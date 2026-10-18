//! Backdoor protection: a base building takes half the damage and mends it
//! while no enemy lane creep is near, and loses the protection for fifteen
//! and a half seconds whenever one is.

use bota_core::{map_of, rules};
use bota_proto::{DamageKind, MapId, Team, Vec2};

use crate::game::{Hit, HitEffect, World};

use super::support::*;

/// What one 400-damage blow of a Dire hero takes off the Radiant melee
/// barracks of the middle lane.
fn blow(world: &mut World, hero: crate::game::Entity, rax: crate::game::Entity) -> i32 {
    world.fill_pools(rax);
    world.hits.push_back(Hit {
        source: Some(hero),
        target: rax,
        amount: 400,
        kind: DamageKind::Physical,
        damage_amp_bp: rules::NOMINAL_BP,
        attack: false,
        pierces: false,
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
}

#[test]
fn only_a_tier_two_tower_holds_it_alone_and_a_tier_one_never_does() {
    let world = World::on_map(map_of(MapId(0)));
    for (tier, index) in [(1, 0), (2, 1), (3, 2)] {
        let tower = structure_at(&world, rules::RADIANT_TOWERS[index].2);
        let held = world.stats.get(tower).expect("settled").backdoor;
        assert_eq!(held, tier >= 2, "tier {tier}");
    }
}
