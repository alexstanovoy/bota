//! The geometry and the damage arithmetic mirrored from the server.

use bota_proto::{Angle, DamageKind, Fixed, Team, Vec2};

use crate::tests::fixtures;
use crate::{
    after_mitigation, facing_gap, facing_towards, heading_of, isqrt, point_along, span, spot_ahead,
    within,
};

#[test]
fn a_facing_is_the_inverse_of_its_heading() {
    for brads in (0..65536).step_by(97) {
        let facing = Angle {
            brads: brads as u16,
        };
        let from = Vec2::from_ints(4000, 4000);
        let there = from + heading_of(facing);
        assert_eq!(facing_towards(from, there), facing, "at {brads} brads");
    }
}

#[test]
fn a_facing_is_exact_on_the_axes_and_the_diagonals() {
    let from = Vec2::ZERO;
    assert_eq!(
        facing_towards(from, Vec2::from_ints(100, 0)),
        Angle { brads: 0 }
    );
    assert_eq!(
        facing_towards(from, Vec2::from_ints(100, 100)),
        Angle { brads: 8192 }
    );
    assert_eq!(
        facing_towards(from, Vec2::from_ints(0, 100)),
        Angle { brads: 16384 }
    );
    assert_eq!(
        facing_towards(from, Vec2::from_ints(-100, 0)),
        Angle { brads: 32768 }
    );
}

#[test]
fn the_gap_between_two_facings_ignores_which_way_round() {
    let one = Angle { brads: 100 };
    let other = Angle { brads: 65500 };
    assert_eq!(facing_gap(one, other), 136);
    assert_eq!(facing_gap(other, one), 136);
}

#[test]
fn a_point_along_a_line_keeps_its_distance() {
    let from = Vec2::from_ints(1000, 1000);
    let towards = Vec2::from_ints(1000, 5000);
    assert_eq!(point_along(from, towards, 700), Vec2::from_ints(1000, 1700));
}

#[test]
fn a_point_along_no_line_at_all_is_where_it_started() {
    let from = Vec2::from_ints(1000, 1000);
    assert_eq!(point_along(from, from, 700), from);
}

#[test]
fn a_raze_lands_its_own_reach_ahead() {
    let mut me = fixtures::hero(1, Team::Radiant, Vec2::from_ints(4000, 4000));
    me.facing = Angle { brads: 16384 };
    let lands = spot_ahead(&me, 450);
    assert_eq!(lands, Vec2::from_ints(4000, 4450));
    let mark = fixtures::creep(2, Team::Dire, Vec2::from_ints(4100, 4600), 550);
    assert!(within(&mark, lands, 250));
    let far = fixtures::creep(3, Team::Dire, Vec2::from_ints(4000, 4800), 550);
    assert!(!within(&far, lands, 250));
}

#[test]
fn armor_takes_its_share_of_a_blow() {
    let mut mark = fixtures::creep(1, Team::Dire, Vec2::ZERO, 550);
    mark.armor = Fixed::from_int(2);
    // A hundred through two armor: 100 * 100 / (100 + 6 * 2).
    assert_eq!(after_mitigation(100, DamageKind::Physical, &mark), 89);
    mark.armor = Fixed::from_int(-5);
    assert_eq!(after_mitigation(100, DamageKind::Physical, &mark), 100);
}

#[test]
fn resistance_takes_its_share_of_a_spell() {
    let mut mark = fixtures::hero(1, Team::Dire, Vec2::ZERO);
    mark.magic_resist = Fixed::from_ratio(1, 4);
    assert_eq!(after_mitigation(200, DamageKind::Magical, &mark), 150);
    assert_eq!(after_mitigation(200, DamageKind::Pure, &mark), 200);
}

#[test]
fn a_square_root_rounds_down() {
    assert_eq!(isqrt(0), 0);
    assert_eq!(isqrt(-4), 0);
    assert_eq!(isqrt(35), 5);
    assert_eq!(isqrt(36), 6);
}

#[test]
fn a_span_is_the_plain_distance() {
    let one = Vec2::from_ints(0, 0);
    let other = Vec2::from_ints(300, 400);
    assert!((span(one, other) - 500.0).abs() < 0.01);
}
