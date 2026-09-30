use crate::profile::{Phase, ScopeGuard, sampled, take_snapshot};

#[test]
fn phase_sampling_selects_only_positive_sixteenth_ticks() {
    for tick in [0, 1, 15, 17, u32::MAX] {
        assert!(!sampled(tick));
    }
    for tick in [16, 32, u32::MAX - 15] {
        assert!(sampled(tick));
    }
}

#[test]
fn phase_scopes_keep_sampled_counts_separate_and_snapshot_drains() {
    let _ = take_snapshot();
    {
        let _outer = ScopeGuard::new(Phase::Targeting, 16, 12);
        let _query = ScopeGuard::new(Phase::TargetQuery, 16, 12);
        let _unsampled = ScopeGuard::new(Phase::TargetQuery, 15, 99);
    }
    let snapshot = take_snapshot();
    assert_eq!(snapshot[Phase::Targeting as usize][0], 1);
    assert_eq!(snapshot[Phase::Targeting as usize][2], 12);
    assert_eq!(snapshot[Phase::TargetQuery as usize][0], 1);
    assert_eq!(snapshot[Phase::TargetQuery as usize][2], 12);
    assert!(take_snapshot().iter().all(|sample| *sample == [0; 3]));
}

#[test]
fn phase_counts_remain_local_to_the_recording_thread() {
    let _ = take_snapshot();
    let child = std::thread::spawn(|| {
        drop(ScopeGuard::new(Phase::Tick, 16, 7));
        take_snapshot()
    })
    .join()
    .expect("profile worker");
    assert_eq!(child[Phase::Tick as usize][0], 1);
    assert_eq!(child[Phase::Tick as usize][2], 7);
    assert!(take_snapshot().iter().all(|sample| *sample == [0; 3]));
}

#[test]
fn fine_movement_phases_keep_independent_sampled_counters() {
    let phases = [
        Phase::MovementIntent,
        Phase::Walk,
        Phase::Separation,
        Phase::BodyIndex,
        Phase::Route,
        Phase::PathQuery,
        Phase::LocalPlan,
        Phase::LocalSearch,
    ];
    let _ = take_snapshot();
    for (index, phase) in phases.into_iter().enumerate() {
        drop(ScopeGuard::new(phase, 16, index + 1));
        drop(ScopeGuard::new(phase, 17, 99));
    }
    let snapshot = take_snapshot();
    for (index, phase) in phases.into_iter().enumerate() {
        assert_eq!(snapshot[phase as usize][0], 1);
        assert_eq!(snapshot[phase as usize][2], (index + 1) as u64);
    }
    assert_eq!(snapshot[Phase::Movement as usize], [0; 3]);
}

#[test]
fn class_zero_target_retention_performs_no_full_search() {
    use crate::game::{MELEE_CREEP, World};
    use bota_proto::{Team, Vec2};

    let mut world = World::new();
    let seeker = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5000, 5000));
    let held = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5100, 5000));
    world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5050, 5000));
    world.settle();
    world.tick = 16;
    world.set_target(seeker, held);
    let reach = world.stats.get(seeker).expect("settled").attack_range;
    assert!(world.reachable(seeker, reach, held));
    let before = world.hash();
    let _ = take_snapshot();

    let selected = world.select_target(seeker, &world.candidates());
    let snapshot = take_snapshot();

    assert_eq!(selected, Some(held));
    assert_eq!(world.hash(), before);
    assert_eq!(snapshot[Phase::TargetQuery as usize][0], 0);
}
