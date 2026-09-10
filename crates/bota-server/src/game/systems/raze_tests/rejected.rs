use bota_proto::{AbilitySlot, DamageKind, Fixed, Target, Team, Vec2};

use crate::game::{PendingCast, Status, StatusKind};

use super::fixtures::*;

#[test]
fn raze_miss_neither_applies_nor_refreshes_a_debuff() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    world.transform.get_mut(target).unwrap().pos = Vec2::from_ints(6000, 5000);
    assert!(world.cast_raze(caster, 0, 1));
    assert!(damage(&world.step(), target).is_empty());
    assert_eq!(effects(&world, target), vec![effect(1, 239)]);
}

#[test]
fn raze_never_damages_or_debuffs_an_ally() {
    let (mut world, caster, target) = fixture();
    world.set_team(target, Team::Radiant);
    assert!(world.cast_raze(caster, 0, 1));
    assert!(damage(&world.step(), target).is_empty());
    assert!(effects(&world, target).is_empty());
}

#[test]
fn raze_invulnerable_at_cast_time_gets_no_damage_or_debuff() {
    let (mut world, caster, target) = fixture();
    world.stats.get_mut(target).unwrap().invulnerable = true;
    assert!(world.cast_raze(caster, 0, 1));
    assert!(damage(&world.step(), target).is_empty());
    assert!(effects(&world, target).is_empty());
}

#[test]
fn raze_invulnerability_at_impact_prevents_a_previously_queued_debuff() {
    let (mut world, caster, target) = fixture();
    assert!(world.cast_raze(caster, 0, 1));
    world.stats.get_mut(target).unwrap().invulnerable = true;
    assert!(damage(&world.step(), target).is_empty());
    assert!(effects(&world, target).is_empty());
}

#[test]
fn raze_shield_applied_before_impact_blocks_damage_without_waiting_for_stats() {
    let (mut world, caster, target) = fixture();
    assert!(world.cast_raze(caster, 0, 1));
    world.statuses.insert(
        target,
        crate::game::Statuses(vec![Status {
            kind: StatusKind::Shielded,
            ticks_left: 30,
        }]),
    );
    assert!(!world.stats.get(target).unwrap().invulnerable);
    assert!(damage(&world.step(), target).is_empty());
    assert!(effects(&world, target).is_empty());
}

#[test]
fn raze_expired_shield_does_not_block_damage_or_debuff() {
    let (mut world, caster, target) = fixture();
    world.statuses.insert(
        target,
        crate::game::Statuses(vec![Status {
            kind: StatusKind::Shielded,
            ticks_left: 1,
        }]),
    );
    assert_eq!(land(&mut world, caster, target, 0), 90);
    assert_eq!(effects(&world, target), vec![effect(1, 240)]);
}

#[test]
fn raze_existing_debuff_is_not_refreshed_by_a_missed_invulnerable_target() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    world.stats.get_mut(target).unwrap().invulnerable = true;
    reveal(&mut world, target);
    assert!(world.cast_raze(caster, 0, 1));
    assert!(damage(&world.step(), target).is_empty());
    assert_eq!(effects(&world, target), vec![effect(1, 239)]);
}

#[test]
fn raze_stacks_do_not_amplify_or_refresh_from_ordinary_magical_damage() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    world.push_hit(Some(caster), target, 90, DamageKind::Magical);
    assert_eq!(damage(&world.step(), target), vec![90]);
    assert_eq!(effects(&world, target), vec![effect(1, 239)]);
}

#[test]
fn raze_zero_damage_from_total_resistance_neither_adds_nor_refreshes_stacks() {
    let (mut world, caster, target) = fixture();
    land(&mut world, caster, target, 0);
    world.stats.get_mut(target).unwrap().magic_resist_pct = 100;
    assert_eq!(land(&mut world, caster, target, 0), 0);
    assert_eq!(effects(&world, target), vec![effect(1, 239)]);
    world.stats.get_mut(target).unwrap().magic_resist_pct = 0;
    assert_eq!(land(&mut world, caster, target, 0), 140);
}

#[test]
fn raze_rounding_to_zero_damage_does_not_create_a_first_stack() {
    let (mut world, caster, target) = fixture();
    world.stats.get_mut(target).unwrap().magic_resist_pct = 99;
    assert_eq!(land(&mut world, caster, target, 0), 0);
    assert!(effects(&world, target).is_empty());
    world.stats.get_mut(target).unwrap().magic_resist_pct = 0;
    assert_eq!(land(&mut world, caster, target, 0), 90);
}

#[test]
fn raze_queued_after_a_lethal_blow_applies_no_debuff() {
    let (mut world, caster, target) = fixture();
    world.push_hit(Some(caster), target, 30000, DamageKind::Pure);
    assert!(world.cast_raze(caster, 0, 1));
    assert!(damage(&world.step(), target).is_empty());
    assert!(world.statuses.get(target).is_none());
    assert!(!world.alive(target));
}

#[test]
fn raze_failed_casts_leave_existing_stacks_unmodified() {
    for failure in 0..4 {
        let (mut world, caster, target) = fixture();
        land(&mut world, caster, target, 0);
        match failure {
            0 => world.abilities.get_mut(caster).unwrap().slots[1].level = 0,
            1 => world.abilities.get_mut(caster).unwrap().slots[1].cooldown = 30,
            2 => world.mana.get_mut(caster).unwrap().mana = Fixed::ZERO,
            3 => {
                world.statuses.insert(
                    caster,
                    crate::game::Statuses(vec![Status {
                        kind: StatusKind::Stunned,
                        ticks_left: 30,
                    }]),
                );
            }
            _ => unreachable!(),
        }
        world.order_cast(
            caster,
            PendingCast {
                slot: AbilitySlot(1),
                target: Target::None,
            },
        );
        assert!(damage(&world.step(), target).is_empty());
        assert_eq!(effects(&world, target), vec![effect(1, 239)]);
    }
}

#[test]
fn raze_dead_caster_cannot_queue_a_hit_or_debuff() {
    let (mut world, caster, target) = fixture();
    world.health.get_mut(caster).unwrap().hp = Fixed::ZERO;
    assert!(!world.cast_raze(caster, 0, 1));
    assert!(world.hits.is_empty());
    assert!(effects(&world, target).is_empty());
}
