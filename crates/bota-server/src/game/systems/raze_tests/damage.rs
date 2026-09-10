use bota_proto::{Angle, Fixed, SlotId, Team};

use super::fixtures::*;

#[test]
fn raze_successive_hits_deal_exact_base_plus_prior_stacks_at_every_level() {
    for (level, amounts) in [
        [90, 140, 190, 240],
        [160, 220, 280, 340],
        [230, 300, 370, 440],
        [300, 380, 460, 540],
    ]
    .into_iter()
    .enumerate()
    {
        let (mut world, caster, target) = fixture();
        for (prior, expected) in amounts.into_iter().enumerate() {
            let before = world.health.get(target).unwrap().hp;
            assert_eq!(land(&mut world, caster, target, level), expected);
            assert_eq!(
                world.health.get(target).unwrap().hp,
                before - Fixed::from_int(expected)
            );
            assert_eq!(effects(&world, target), vec![effect(prior as u32 + 1, 240)]);
        }
    }
}

#[test]
fn raze_all_three_reaches_share_stacks_even_when_hits_resolve_together() {
    let (mut world, caster, target) = fixture();
    for reach in 0..3 {
        assert!(world.cast_raze(caster, 0, reach));
    }
    assert_eq!(damage(&world.step(), target), vec![90, 140, 190]);
    assert_eq!(effects(&world, target), vec![effect(3, 240)]);
}

#[test]
fn raze_stack_bonus_uses_the_current_cast_level() {
    let (mut world, caster, target) = fixture();
    assert_eq!(land(&mut world, caster, target, 0), 90);
    assert_eq!(land(&mut world, caster, target, 3), 380);
    assert_eq!(land(&mut world, caster, target, 1), 280);
}

#[test]
fn raze_magic_resistance_reduces_the_total_after_adding_stack_bonus() {
    let (mut world, caster, target) = fixture();
    world.stats.get_mut(target).unwrap().magic_resist_pct = 25;
    let amounts: Vec<i32> = (0..4)
        .map(|_| land(&mut world, caster, target, 0))
        .collect();
    assert_eq!(amounts, vec![67, 105, 142, 180]);
    assert_eq!(effects(&world, target), vec![effect(4, 240)]);
}

#[test]
fn raze_same_team_casters_keep_independent_damage_stacks() {
    let (mut world, first, target) = fixture();
    let second = hero(&mut world, Team::Radiant, ORIGIN, SlotId(2));
    assert_eq!(land(&mut world, first, target, 0), 90);
    assert_eq!(land(&mut world, second, target, 0), 90);
    assert_eq!(land(&mut world, first, target, 0), 140);
    assert_eq!(land(&mut world, second, target, 0), 140);
    assert_eq!(
        effects(&world, target),
        vec![effect(2, 239), effect(2, 240)]
    );
}

#[test]
fn raze_opposing_sides_stack_independently_on_a_neutral_target() {
    let (mut world, first, target) = fixture();
    world.set_team(target, Team::Neutral);
    let second = hero(&mut world, Team::Dire, ORIGIN, SlotId(2));
    assert_eq!(land(&mut world, first, target, 0), 90);
    assert_eq!(land(&mut world, second, target, 0), 90);
    assert_eq!(land(&mut world, first, target, 0), 140);
    assert_eq!(land(&mut world, second, target, 0), 140);
}

#[test]
fn raze_dire_casts_along_its_facing_with_the_same_bonus() {
    let (mut world, target, caster) = fixture();
    world.transform.get_mut(caster).unwrap().facing = Angle { brads: 32768 };
    assert_eq!(land(&mut world, caster, target, 2), 230);
    assert_eq!(land(&mut world, caster, target, 2), 300);
}

#[test]
fn raze_stacks_saturate_at_255_not_at_three_and_continue_refreshing() {
    let (mut world, caster, target) = fixture();
    for hit in 0..257 {
        world.fill_pools(target);
        assert_eq!(land(&mut world, caster, target, 3), 300 + 80 * hit.min(255));
        assert_eq!(
            effects(&world, target),
            vec![effect((hit + 1).min(255) as u32, 240)]
        );
    }
}
