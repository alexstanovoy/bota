//! Shop queries can receive signed Q16.16 unit positions from an untrusted snapshot.

use bota_core::segment_distance_squared;
use bota_proto::{Fixed, Vec2};

#[test]
fn shop_segment_projection_handles_extreme_wire_coordinates_without_overflow() {
    let point = |x, y| Vec2 {
        x: Fixed { raw: x },
        y: Fixed { raw: y },
    };
    let low = i32::MIN;
    let high = i32::MAX;
    // The projection lies halfway along the diagonal. The cross-product square
    // exceeds i128::MAX, although its quotient is a representable squared distance.
    let expected = (u128::from(u32::MAX).pow(2) / 2) as i64;
    assert_eq!(
        segment_distance_squared(point(low, high), point(low, low), point(high, high)),
        expected
    );
}
