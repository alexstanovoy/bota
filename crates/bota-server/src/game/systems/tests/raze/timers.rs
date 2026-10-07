use super::fixtures::*;

#[test]
fn raze_debuff_remains_for_239_ticks_and_expires_on_tick_240() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    for _ in 0..239 {
        world.tick_gear();
    }
    assert_eq!(effects(&world, target), vec![effect(1, 1)]);
    world.tick_gear();
    assert!(effects(&world, target).is_empty());
    assert_eq!(land(&mut world, caster, target, 0), 90);
}

#[test]
fn raze_hit_one_tick_before_expiry_gets_bonus_and_refreshes_the_whole_stack() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    for _ in 0..238 {
        world.tick_gear();
    }
    assert_eq!(land(&mut world, caster, target, 0), 140);
    assert_eq!(effects(&world, target), vec![effect(2, 240)]);
    for _ in 0..239 {
        world.tick_gear();
    }
    assert_eq!(effects(&world, target), vec![effect(2, 1)]);
}

#[test]
fn raze_hit_at_expiry_uses_base_damage_and_starts_a_new_stack() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    for _ in 0..239 {
        world.tick_gear();
    }
    assert_eq!(land(&mut world, caster, target, 0), 90);
    assert_eq!(effects(&world, target), vec![effect(1, 240)]);
}

#[test]
fn raze_refresh_does_not_leave_an_independent_timer_for_each_hit() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    for _ in 0..100 {
        world.tick_gear();
    }
    assert_eq!(land(&mut world, caster, target, 0), 140);
    for _ in 0..140 {
        world.tick_gear();
    }
    assert_eq!(effects(&world, target), vec![effect(2, 100)]);
    assert_eq!(land(&mut world, caster, target, 0), 190);
}
