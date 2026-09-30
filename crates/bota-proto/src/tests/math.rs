//! Fixed-point arithmetic.

use crate::*;

const HALF: Fixed = Fixed::from_ratio(1, 2);
const QUARTER: Fixed = Fixed::from_ratio(1, 4);

#[test]
fn whole_units_survive_a_round_trip() {
    for units in [-32768, -1000, -1, 0, 1, 1000, 8192, 32767] {
        assert_eq!(Fixed::from_int(units).to_int(), units);
    }
}

#[test]
fn one_is_one_shifted() {
    assert_eq!(Fixed::ONE, Fixed::from_int(1));
    assert_eq!(Fixed::ONE.raw, 1 << Fixed::FRAC_BITS);
}

#[test]
fn ratios_are_exact_when_the_denominator_is_a_power_of_two() {
    assert_eq!(HALF.raw, 32768);
    assert_eq!(QUARTER.raw, 16384);
    assert_eq!(HALF + HALF, Fixed::ONE);
    assert_eq!(QUARTER * Fixed::from_int(4), Fixed::ONE);
}

#[test]
fn addition_and_subtraction_are_inverses() {
    let a = Fixed::from_int(1234);
    let b = Fixed::from_ratio(7, 8);
    assert_eq!(a + b - b, a);
    assert_eq!(a - a, Fixed::ZERO);
    assert_eq!(-(-a), a);
}

#[test]
fn assignment_operators_match_their_binary_forms() {
    let mut a = Fixed::from_int(10);
    a += HALF;
    assert_eq!(a, Fixed::from_int(10) + HALF);
    a -= HALF;
    assert_eq!(a, Fixed::from_int(10));
}

#[test]
fn multiplication_keeps_the_scale() {
    assert_eq!(Fixed::from_int(7) * Fixed::ONE, Fixed::from_int(7));
    assert_eq!(Fixed::from_int(7) * Fixed::ZERO, Fixed::ZERO);
    assert_eq!(HALF * HALF, QUARTER);
    assert_eq!(Fixed::from_int(200) * HALF, Fixed::from_int(100));
    assert_eq!(Fixed::from_int(-200) * HALF, Fixed::from_int(-100));
}

#[test]
fn multiplication_of_map_scale_values_does_not_overflow_the_intermediate() {
    // The raw product needs 44 bits; the i64 intermediate keeps it exact.
    let far = Fixed::from_int(8000);
    assert_eq!(far * HALF, Fixed::from_int(4000));
}

#[test]
fn division_inverts_multiplication() {
    let a = Fixed::from_int(360);
    assert_eq!(a / Fixed::ONE, a);
    assert_eq!(a / Fixed::from_int(2), Fixed::from_int(180));
    assert_eq!(Fixed::ONE / Fixed::from_int(4), QUARTER);
    assert_eq!((a / Fixed::from_int(8)) * Fixed::from_int(8), a);
}

#[test]
fn rounding_goes_towards_negative_infinity() {
    // An arithmetic shift floors rather than truncating towards zero.
    assert_eq!(Fixed { raw: -1 }.to_int(), -1);
    assert_eq!(Fixed { raw: 1 }.to_int(), 0);
    assert_eq!(Fixed::from_ratio(-1, 2).to_int(), -1);

    let tiny = Fixed { raw: 1 };
    assert_eq!(tiny * HALF, Fixed::ZERO);
    assert_eq!(-tiny * HALF, Fixed { raw: -1 });
}

#[test]
fn frac_is_always_the_positive_remainder() {
    assert_eq!(Fixed::from_int(3).frac(), Fixed::ZERO);
    assert_eq!((Fixed::from_int(3) + QUARTER).frac(), QUARTER);
    assert_eq!((Fixed::from_int(-3) + QUARTER).frac(), QUARTER);
}

#[test]
fn abs_removes_the_sign() {
    assert_eq!(Fixed::from_int(-5).abs(), Fixed::from_int(5));
    assert_eq!(Fixed::from_int(5).abs(), Fixed::from_int(5));
    assert_eq!(Fixed::ZERO.abs(), Fixed::ZERO);
}

#[test]
fn checked_operations_refuse_to_overflow() {
    assert_eq!(Fixed::MAX.checked_add(Fixed::EPSILON), None);
    assert_eq!(Fixed::MIN.checked_sub(Fixed::EPSILON), None);
    assert_eq!(Fixed::MAX.checked_mul(Fixed::from_int(2)), None);
    assert_eq!(Fixed::from_int(1).checked_div(Fixed::ZERO), None);

    assert_eq!(
        Fixed::from_int(2).checked_add(Fixed::from_int(3)),
        Some(Fixed::from_int(5))
    );
    assert_eq!(
        Fixed::from_int(6).checked_div(Fixed::from_int(3)),
        Some(Fixed::from_int(2))
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "fixed-point overflow")]
fn overflow_panics_in_debug() {
    let _ = Fixed::MAX + Fixed::EPSILON;
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "fixed-point overflow")]
fn from_int_beyond_the_range_panics_in_debug() {
    let _ = Fixed::from_int(40_000);
}

#[cfg(not(debug_assertions))]
#[test]
fn overflow_saturates_in_release() {
    assert_eq!(Fixed::MAX + Fixed::EPSILON, Fixed::MAX);
    assert_eq!(Fixed::MIN - Fixed::EPSILON, Fixed::MIN);
    assert_eq!(Fixed::MAX * Fixed::from_int(2), Fixed::MAX);
    assert_eq!(Fixed::from_int(40_000), Fixed::MAX);
    assert_eq!(Fixed::from_int(-40_000), Fixed::MIN);
}

#[test]
fn display_reads_as_a_decimal() {
    assert_eq!(Fixed::ZERO.to_string(), "0.00000");
    assert_eq!(Fixed::ONE.to_string(), "1.00000");
    assert_eq!(HALF.to_string(), "0.50000");
    assert_eq!(Fixed::from_int(-1).to_string(), "-1.00000");
    assert_eq!((-HALF).to_string(), "-0.50000");
    assert_eq!(Fixed::from_int(4096).to_string(), "4096.00000");
    assert_eq!(Fixed::EPSILON.to_string(), "0.00001");
}

#[test]
fn debug_matches_display() {
    let v = -HALF;
    assert_eq!(format!("{v:?}"), v.to_string());
}

#[test]
fn to_f32_matches_the_scale() {
    assert_eq!(Fixed::ONE.to_f32(), 1.0);
    assert_eq!(HALF.to_f32(), 0.5);
    assert_eq!(Fixed::from_int(-3).to_f32(), -3.0);
}

#[test]
fn vec2_arithmetic_is_componentwise() {
    let a = Vec2::from_ints(10, 20);
    let b = Vec2::from_ints(3, 4);

    assert_eq!(a + b, Vec2::from_ints(13, 24));
    assert_eq!(a - b, Vec2::from_ints(7, 16));
    assert_eq!(-b, Vec2::from_ints(-3, -4));
    assert_eq!(b * Fixed::from_int(2), Vec2::from_ints(6, 8));
    assert_eq!(
        b * HALF,
        Vec2 {
            x: Fixed::from_ratio(3, 2),
            y: Fixed::from_int(2)
        }
    );

    let mut c = a;
    c += b;
    assert_eq!(c, a + b);
    c -= b;
    assert_eq!(c, a);
}

#[test]
fn length_squared_is_the_pythagorean_sum() {
    let v = Vec2::from_ints(3, 4);
    assert_eq!(v.length_squared(), Fixed::from_int(5).squared_raw());
    assert_eq!(Vec2::ZERO.length_squared(), 0);
}

#[test]
fn distance_squared_spans_the_whole_map_without_overflow() {
    // Opposite corners of an 8192-unit square: the squared distance
    // overflows a Fixed.
    let a = Vec2::ZERO;
    let b = Vec2::from_ints(8192, 8192);

    let expected = 2 * Fixed::from_int(8192).squared_raw();
    assert_eq!(a.distance_squared(b), expected);
    assert_eq!(b.distance_squared(a), expected);
    assert!(expected > i32::MAX as i64, "the point of keeping this raw");
    assert!(expected < i64::MAX / 2, "and it still has room to spare");
}

#[test]
fn within_compares_against_a_radius() {
    let origin = Vec2::ZERO;
    let target = Vec2::from_ints(300, 400);
    let exact = Fixed::from_int(500);

    assert!(
        origin.within(target, exact),
        "on the boundary counts as within"
    );
    assert!(origin.within(target, exact + Fixed::EPSILON));
    assert!(!origin.within(target, exact - Fixed::EPSILON));
    assert!(origin.within(origin, Fixed::ZERO));
}

#[test]
fn within_holds_at_attack_range_across_the_map() {
    let tower = Vec2::from_ints(7000, 7000);
    let hero = Vec2::from_ints(7000, 6300);
    let range = Fixed::from_int(700);

    assert!(tower.within(hero, range));
    assert!(!tower.within(hero, range - Fixed::EPSILON));
}
