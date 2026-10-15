use super::fixtures::*;

#[test]
fn raze_debuff_remains_for_179_ticks_and_expires_on_tick_180() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    for _ in 0..179 {
        world.tick_gear();
    }
    assert_eq!(effects(&world, target), vec![effect(1, 1)]);
    world.tick_gear();
    assert!(effects(&world, target).is_empty());
    assert_eq!(land(&mut world, caster, target, 0), 85);
}

#[test]
fn raze_hit_one_tick_before_expiry_gets_bonus_and_refreshes_the_whole_stack() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    for _ in 0..178 {
        world.tick_gear();
    }
    assert_eq!(land(&mut world, caster, target, 0), 120);
    assert_eq!(effects(&world, target), vec![effect(2, 180)]);
    for _ in 0..179 {
        world.tick_gear();
    }
    assert_eq!(effects(&world, target), vec![effect(2, 1)]);
}

#[test]
fn raze_hit_at_expiry_uses_base_damage_and_starts_a_new_stack() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    for _ in 0..179 {
        world.tick_gear();
    }
    assert_eq!(land(&mut world, caster, target, 0), 85);
    assert_eq!(effects(&world, target), vec![effect(1, 180)]);
}

#[test]
fn raze_refresh_does_not_leave_an_independent_timer_for_each_hit() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    for _ in 0..100 {
        world.tick_gear();
    }
    assert_eq!(land(&mut world, caster, target, 0), 120);
    for _ in 0..140 {
        world.tick_gear();
    }
    assert_eq!(effects(&world, target), vec![effect(2, 40)]);
    assert_eq!(land(&mut world, caster, target, 0), 155);
}
