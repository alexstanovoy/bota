//! The world fingerprint and the match randomness it covers.

use crate::game::{MELEE_CREEP, World};
use bota_proto::Fixed;

use super::support::*;

#[test]
fn global_random_streams_advance_between_draws() {
    let mut rng = crate::game::MatchRng::new(&[7; 32], 11);
    let first = rng.global(crate::game::Purpose::Wave).next_u32();
    let second = rng.global(crate::game::Purpose::Wave).next_u32();

    assert_ne!(
        first, second,
        "a global stream must not restart for every draw"
    );
}

#[test]
fn world_hash_changes_when_hidden_random_state_advances() {
    let mut world = World::new();
    let before = world.hash();

    world.rng.global(crate::game::Purpose::Wave).next_u32();

    assert_ne!(world.hash(), before);
}

#[test]
fn world_hash_changes_when_uphill_prd_state_advances() {
    let mut world = World::new();
    let stream = world.rng.for_unit(
        crate::game::Purpose::Evasion,
        bota_proto::EntityId {
            idx: 0,
            generation: 1,
        },
        0,
    );
    world
        .uphill_miss
        .push(Some(crate::game::PseudoRandom25::new(stream)));
    let before = world.hash();

    world.uphill_miss[0].as_mut().expect("chance").roll();

    assert_ne!(world.hash(), before);
}

#[test]
fn world_hash_includes_projectile_uphill_state() {
    let level = world_with_projectile_uphill_state(1, true);
    let higher_launch = world_with_projectile_uphill_state(2, true);
    let cannot_miss = world_with_projectile_uphill_state(1, false);

    assert_ne!(level.hash(), higher_launch.hash());
    assert_ne!(level.hash(), cannot_miss.hash());
}

#[test]
fn the_fingerprint_moves_when_the_world_does() {
    let mut world = World::for_match(&config(), config().rng());
    let before = world.hash();
    world.step();
    assert_ne!(before, world.hash(), "a tick is a change");
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(9000, 9216),
    );
    world.settle();
    let with = world.hash();
    if let Some(health) = world.health.get_mut(creep) {
        health.hp -= Fixed::ONE;
    }
    assert_ne!(with, world.hash(), "so is a point of health");
}
