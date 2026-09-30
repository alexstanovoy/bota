//! Keeping a target in reach against a closer one of the same class.

use bota_proto::{Fixed, Team, Vec2};

use crate::game::{Entity, LaneAi, MELEE_CREEP, SIEGE_CREEP, Visibility, World, tower_def};

#[test]
fn reachable_class_zero_target_is_kept_over_a_closer_equal_class() {
    for siege in [false, true] {
        let (world, seeker, held, closer) = retention_world(siege, true);
        let reach = world.stats.get(seeker).expect("settled").attack_range;
        assert_eq!(
            world.best_valid_in_range(seeker, reach, &world.candidates()),
            Some(closer)
        );
        let before = world.hash();

        assert_eq!(world.select_target(seeker, &world.candidates()), Some(held));

        assert_eq!(world.hash(), before);
    }
}

#[test]
fn reachable_nonzero_class_target_is_replaced_by_a_better_class() {
    for siege in [false, true] {
        let (world, seeker, _, better) = retention_world(siege, false);
        let before = world.hash();

        assert_eq!(
            world.select_target(seeker, &world.candidates()),
            Some(better)
        );

        assert_eq!(world.hash(), before);
    }
}

#[test]
fn unavailable_class_zero_target_does_not_bypass_reacquisition() {
    for condition in 0..4 {
        let (mut world, seeker, held, closer) = retention_world(false, true);
        match condition {
            0 => world.health.get_mut(held).expect("health").hp = Fixed::ZERO,
            1 => {
                world.visibility.insert(held, Visibility::NONE);
            }
            2 => world.transform.get_mut(held).expect("position").pos = Vec2::from_ints(9000, 9000),
            3 => {
                assert!(world.despawn(held));
                let replacement =
                    world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(9000, 9000));
                assert_eq!(replacement.index(), held.index());
                assert_ne!(replacement.generation(), held.generation());
            }
            _ => unreachable!(),
        }
        let before = world.hash();

        assert_eq!(
            world.select_target(seeker, &world.candidates()),
            Some(closer)
        );

        assert_eq!(world.hash(), before);
    }
}

#[test]
fn active_retention_lock_precedes_better_class_replacement() {
    let (mut world, seeker, held, _) = retention_world(true, false);
    world.lane_ai.insert(
        seeker,
        LaneAi {
            last_seen: None,
            keep_until: world.tick + 1,
            roused_by: None,
            roused_at_own: false,
            chase_until: 0,
        },
    );
    let before = world.hash();

    assert_eq!(world.select_target(seeker, &world.candidates()), Some(held));

    assert_eq!(world.hash(), before);
}

fn retention_world(siege: bool, class_zero: bool) -> (World, Entity, Entity, Entity) {
    let mut world = World::new();
    let seeker_kind = if siege { &SIEGE_CREEP } else { &MELEE_CREEP };
    let preferred = if siege { tower_def(1) } else { &MELEE_CREEP };
    let held_kind = if class_zero {
        preferred
    } else if siege {
        &MELEE_CREEP
    } else {
        &SIEGE_CREEP
    };
    let seeker = world.spawn_unit(seeker_kind, Team::Radiant, Vec2::from_ints(5000, 5000));
    let held = world.spawn_unit(held_kind, Team::Dire, Vec2::from_ints(5100, 5000));
    let closer = world.spawn_unit(preferred, Team::Dire, Vec2::from_ints(5050, 5000));
    world.settle();
    world.tick = 16;
    world.set_target(seeker, held);
    let reach = world.stats.get(seeker).expect("settled").attack_range;
    assert!(world.reachable(seeker, reach, held));
    assert!(world.reachable(seeker, reach, closer));
    (world, seeker, held, closer)
}
