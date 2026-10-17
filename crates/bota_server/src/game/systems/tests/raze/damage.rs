use bota_proto::{Angle, Fixed, SlotId, Team};

use super::fixtures::*;

#[test]
fn raze_successive_hits_deal_exact_base_plus_prior_stacks_at_every_level() {
    for (level, amounts) in [
        [85, 120, 155, 190],
        [150, 200, 250, 300],
        [215, 280, 345, 410],
        [280, 360, 440, 520],
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
            assert_eq!(effects(&world, target), vec![effect(prior as u32 + 1, 180)]);
        }
    }
}

#[test]
fn raze_all_three_reaches_share_stacks_even_when_hits_resolve_together() {
    let (mut world, caster, target) = fixture();
    for reach in 0..3 {
        assert!(world.cast_raze(caster, 0, reach));
    }
    assert_eq!(burns_on(&world.step(), target), vec![85, 120, 155]);
    assert_eq!(effects(&world, target), vec![effect(3, 180)]);
}

#[test]
fn raze_stack_bonus_uses_the_current_cast_level() {
    let (mut world, caster, target) = fixture();
    assert_eq!(land(&mut world, caster, target, 0), 85);
    assert_eq!(land(&mut world, caster, target, 3), 360);
    assert_eq!(land(&mut world, caster, target, 1), 250);
}

#[test]
fn raze_magic_resistance_reduces_the_total_after_adding_stack_bonus() {
    let (mut world, caster, target) = fixture();
    world.stats.get_mut(target).unwrap().magic_resist_bp = 2500;
    let amounts: Vec<i32> = (0..4)
        .map(|_| land(&mut world, caster, target, 0))
        .collect();
    assert_eq!(amounts, vec![63, 90, 116, 142]);
    assert_eq!(effects(&world, target), vec![effect(4, 180)]);
}

#[test]
fn raze_outgoing_amplification_multiplies_base_and_prior_stacks_together() {
    let (mut amplified, caster, target) = fixture();
    assert_eq!(land(&mut amplified, caster, target, 0), 85);
    assert_eq!(land(&mut amplified, caster, target, 0), 120);
    amplified.stats.get_mut(caster).unwrap().magic_amp_bp = 15_000;
    assert_eq!(
        land(&mut amplified, caster, target, 0),
        (85 + 2 * 35) * 15_000 / 10_000,
        "the stack composition is multiplied once as a whole"
    );

    let (mut plain, caster, target) = fixture();
    assert_eq!(land(&mut plain, caster, target, 0), 85);
    assert_eq!(land(&mut plain, caster, target, 0), 120);
    assert_eq!(land(&mut plain, caster, target, 0), 85 + 2 * 35);
}

#[test]
fn raze_casters_of_one_side_or_both_sides_on_a_neutral_keep_independent_stacks() {
    for (second_side, neutral) in [(Team::Radiant, false), (Team::Dire, true)] {
        let (mut world, first, target) = fixture();
        if neutral {
            world.set_team(target, Team::Neutral);
        }
        let second = hero(&mut world, second_side, ORIGIN, SlotId(2));
        for expected in [85, 120] {
            assert_eq!(land(&mut world, first, target, 0), expected);
            assert_eq!(land(&mut world, second, target, 0), expected);
        }
        assert_eq!(
            effects(&world, target),
            vec![effect(2, 179), effect(2, 180)]
        );
    }
}

#[test]
fn raze_dire_casts_along_its_facing_with_the_same_bonus() {
    let (mut world, target, caster) = fixture();
    world.transform.get_mut(caster).unwrap().facing = Angle { brads: 32768 };
    assert_eq!(land(&mut world, caster, target, 2), 215);
    assert_eq!(land(&mut world, caster, target, 2), 280);
}

#[test]
fn raze_stacks_saturate_at_255_not_at_three_and_continue_refreshing() {
    let (mut world, caster, target) = fixture();
    for hit in 0..257 {
        world.fill_pools(target);
        assert_eq!(land(&mut world, caster, target, 3), 280 + 80 * hit.min(255));
        assert_eq!(
            effects(&world, target),
            vec![effect((hit + 1).min(255) as u32, 180)]
        );
    }
}
