//! Deterministic integer geometry: stepping, turning, distances.

use bota_proto::{Angle, Fixed, Vec2};

use crate::game::rules;

/// Integer square root, rounded down.
///
/// The one place a length is ever taken; everything else compares squares.
pub fn isqrt64(n: i64) -> i64 {
    debug_assert!(n >= 0, "no square root of a negative");
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut next = (x + 1) / 2;
    while next < x {
        x = next;
        next = (x + n / x) / 2;
    }
    x
}

/// The distance covered in one tick at a per-second speed.
pub fn per_tick(speed: Fixed) -> Fixed {
    Fixed {
        raw: speed.raw / rules::TICKS_PER_SECOND as i32,
    }
}

/// One movement step from `pos` towards `target`, no further than `step`.
pub fn move_towards(pos: Vec2, target: Vec2, step: Fixed) -> Vec2 {
    let dx = i64::from(target.x.raw) - i64::from(pos.x.raw);
    let dy = i64::from(target.y.raw) - i64::from(pos.y.raw);
    let dist = isqrt64(dx * dx + dy * dy);
    if dist <= i64::from(step.raw) {
        return target;
    }
    let sx = dx * i64::from(step.raw) / dist;
    let sy = dy * i64::from(step.raw) / dist;
    Vec2 {
        x: Fixed {
            raw: pos.x.raw.saturating_add(sx as i32),
        },
        y: Fixed {
            raw: pos.y.raw.saturating_add(sy as i32),
        },
    }
}

/// The point `distance` away from `from` along the line towards `towards`.
///
/// `from` itself when the two stand on the same spot.
pub fn point_along(from: Vec2, towards: Vec2, distance: Fixed) -> Vec2 {
    let dx = i64::from(towards.x.raw) - i64::from(from.x.raw);
    let dy = i64::from(towards.y.raw) - i64::from(from.y.raw);
    let span = isqrt64(dx * dx + dy * dy);
    if span == 0 {
        return from;
    }
    let sx = dx * i64::from(distance.raw) / span;
    let sy = dy * i64::from(distance.raw) / span;
    Vec2 {
        x: Fixed {
            raw: from.x.raw.saturating_add(sx as i32),
        },
        y: Fixed {
            raw: from.y.raw.saturating_add(sy as i32),
        },
    }
}

/// The facing from one position towards another, in brads.
///
/// A piecewise-linear octant approximation: exact on the axes and diagonals,
/// within a few degrees elsewhere. [`heading_of`] is its inverse.
pub fn facing_towards(from: Vec2, to: Vec2) -> Angle {
    let dx = i64::from(to.x.raw) - i64::from(from.x.raw);
    let dy = i64::from(to.y.raw) - i64::from(from.y.raw);
    if dx == 0 && dy == 0 {
        return Angle { brads: 0 };
    }
    let (adx, ady) = (dx.abs(), dy.abs());
    // 8192 brads per octant; the slope maps linearly onto one octant.
    let slope = if adx >= ady {
        (ady << 13) / adx
    } else {
        (adx << 13) / ady
    };
    let octant = match (dx >= 0, dy >= 0, adx >= ady) {
        (true, true, true) => slope,
        (true, true, false) => 16384 - slope,
        (false, true, false) => 16384 + slope,
        (false, true, true) => 32768 - slope,
        (false, false, true) => 32768 + slope,
        (false, false, false) => 49152 - slope,
        (true, false, false) => 49152 + slope,
        (true, false, true) => 65536 - slope,
    };
    Angle {
        brads: (octant & 0xFFFF) as u16,
    }
}

/// An offset pointing where a facing looks, in the octant mapping of
/// [`facing_towards`].
///
/// The inverse of [`facing_towards`]: the facing from any point towards that
/// point plus this offset is the angle handed in. The offset is direction
/// only; its length is one octant span of world units, and never zero.
pub fn heading_of(facing: Angle) -> Vec2 {
    let brads = i32::from(facing.brads);
    let slope = brads % 8192;
    let (dx, dy) = match brads / 8192 {
        0 => (8192, slope),
        1 => (8192 - slope, 8192),
        2 => (-slope, 8192),
        3 => (-8192, 8192 - slope),
        4 => (-8192, -slope),
        5 => (-(8192 - slope), -8192),
        6 => (slope, -8192),
        _ => (8192, -(8192 - slope)),
    };
    Vec2::from_ints(dx, dy)
}

/// Keeps a position on the map.
pub fn clamp_to_map(pos: Vec2) -> Vec2 {
    let max = Fixed::from_int(rules::MAP_SIZE);
    Vec2 {
        x: Fixed {
            raw: pos.x.raw.clamp(0, max.raw),
        },
        y: Fixed {
            raw: pos.y.raw.clamp(0, max.raw),
        },
    }
}

/// The shortest signed rotation from one facing to another, in brads.
///
/// An exactly opposite facing turns counter-clockwise.
pub fn angle_delta(from: Angle, to: Angle) -> i32 {
    let d = (i32::from(to.brads) - i32::from(from.brads)) & 0xFFFF;
    if d > 32768 { d - 65536 } else { d }
}

/// One tick of turning from a facing towards another, clamped by the rate.
pub fn turn_towards(from: Angle, to: Angle, rate: u16) -> Angle {
    let clamped = angle_delta(from, to).clamp(-i32::from(rate), i32::from(rate));
    Angle {
        brads: (i32::from(from.brads) + clamped) as u16,
    }
}

/// How far a facing is from another, in brads, ignoring direction.
pub fn facing_gap(a: Angle, b: Angle) -> u16 {
    angle_delta(a, b).unsigned_abs() as u16
}

/// Ticks a body turning at a rate stands before it may walk off in a new
/// direction: it walks once within [`rules::TURN_TOLERANCE_BRADS`] of it.
pub fn stall_ticks(facing: Angle, wanted: Angle, rate: u16) -> u32 {
    let gap = u32::from(facing_gap(facing, wanted));
    let tolerance = u32::from(rules::TURN_TOLERANCE_BRADS);
    if gap <= tolerance {
        return 0;
    }
    let rate = u32::from(rate).max(1);
    (gap - tolerance).div_ceil(rate)
}

/// Squared distance from a point to a segment, in the raw units of
/// [`Vec2::distance_squared`].
pub fn segment_distance_squared(p: Vec2, a: Vec2, b: Vec2) -> i64 {
    let apx = i64::from(p.x.raw) - i64::from(a.x.raw);
    let apy = i64::from(p.y.raw) - i64::from(a.y.raw);
    let abx = i64::from(b.x.raw) - i64::from(a.x.raw);
    let aby = i64::from(b.y.raw) - i64::from(a.y.raw);
    let dot = apx * abx + apy * aby;
    if dot <= 0 {
        return p.distance_squared(a);
    }
    let len2 = abx * abx + aby * aby;
    if dot >= len2 {
        return p.distance_squared(b);
    }
    let cross = apx * aby - apy * abx;
    (i128::from(cross) * i128::from(cross) / i128::from(len2)) as i64
}

/// The nearest point of a segment.
pub fn segment_nearest(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let apx = i64::from(p.x.raw) - i64::from(a.x.raw);
    let apy = i64::from(p.y.raw) - i64::from(a.y.raw);
    let abx = i64::from(b.x.raw) - i64::from(a.x.raw);
    let aby = i64::from(b.y.raw) - i64::from(a.y.raw);
    let dot = apx * abx + apy * aby;
    let len2 = abx * abx + aby * aby;
    if dot <= 0 || len2 == 0 {
        return a;
    }
    if dot >= len2 {
        return b;
    }
    let x = i64::from(a.x.raw) + (i128::from(abx) * i128::from(dot) / i128::from(len2)) as i64;
    let y = i64::from(a.y.raw) + (i128::from(aby) * i128::from(dot) / i128::from(len2)) as i64;
    Vec2 {
        x: Fixed { raw: x as i32 },
        y: Fixed { raw: y as i32 },
    }
}

/// Squared distance from a point to an axis-aligned box given by its low
/// and high corners, in the raw units of [`Vec2::distance_squared`]. Zero
/// inside the box.
pub fn point_box_distance_squared(p: Vec2, lo: Vec2, hi: Vec2) -> i64 {
    let px = i64::from(p.x.raw);
    let py = i64::from(p.y.raw);
    let dx = (i64::from(lo.x.raw) - px)
        .max(px - i64::from(hi.x.raw))
        .max(0);
    let dy = (i64::from(lo.y.raw) - py)
        .max(py - i64::from(hi.y.raw))
        .max(0);
    dx * dx + dy * dy
}

/// Whether a segment touches an axis-aligned box.
pub fn segment_hits_box(a: Vec2, b: Vec2, lo: Vec2, hi: Vec2) -> bool {
    if a.x.max(b.x) < lo.x || a.x.min(b.x) > hi.x || a.y.max(b.y) < lo.y || a.y.min(b.y) > hi.y {
        return false;
    }
    let dx = i64::from(b.x.raw) - i64::from(a.x.raw);
    let dy = i64::from(b.y.raw) - i64::from(a.y.raw);
    if dx == 0 && dy == 0 {
        return true;
    }
    // Along the segment's normal the segment is one point; the box
    // straddles it or misses it.
    let (mut least, mut most) = (i128::MAX, i128::MIN);
    for (cx, cy) in [(lo.x, lo.y), (hi.x, lo.y), (hi.x, hi.y), (lo.x, hi.y)] {
        let rx = i128::from(cx.raw) - i128::from(a.x.raw);
        let ry = i128::from(cy.raw) - i128::from(a.y.raw);
        let side = -i128::from(dy) * rx + i128::from(dx) * ry;
        least = least.min(side);
        most = most.max(side);
    }
    least <= 0 && most >= 0
}

/// Squared distance from a segment to an axis-aligned box, in the raw units
/// of [`Vec2::distance_squared`]. Zero where they touch.
pub fn segment_box_distance_squared(a: Vec2, b: Vec2, lo: Vec2, hi: Vec2) -> i64 {
    if segment_hits_box(a, b, lo, hi) {
        return 0;
    }
    let mut best = point_box_distance_squared(a, lo, hi).min(point_box_distance_squared(b, lo, hi));
    for corner in [lo, Vec2 { x: hi.x, y: lo.y }, hi, Vec2 { x: lo.x, y: hi.y }] {
        best = best.min(segment_distance_squared(corner, a, b));
    }
    best
}
