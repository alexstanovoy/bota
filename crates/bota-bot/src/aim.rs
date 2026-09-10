//! Geometry and damage arithmetic the wire does not carry.
//!
//! The server decides which way a body looks, where a raze lands and what a
//! blow is worth after armor. A bot that wants to know before it asks works
//! the same sums here. The geometry is the server's own, integer for integer;
//! the mitigation is worked from the rounded stats a view carries, so it is a
//! forecast rather than the number the server will reach.

use bota_proto::{Angle, DamageKind, Fixed, UnitView, Vec2};

/// Integer square root, rounded down.
pub fn isqrt(n: i64) -> i64 {
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

/// How far apart two spots are, in world units.
pub fn span(one: Vec2, other: Vec2) -> f32 {
    let dx = one.x.to_f32() - other.x.to_f32();
    let dy = one.y.to_f32() - other.y.to_f32();
    (dx * dx + dy * dy).sqrt()
}

/// The ground between two bodies, edge to edge. Negative when they overlap.
pub fn gap_between(one: &UnitView, other: &UnitView) -> f32 {
    span(one.pos, other.pos) - one.radius.to_f32() - other.radius.to_f32()
}

/// The facing from one spot towards another, in brads.
///
/// A piecewise-linear octant approximation, exact on the axes and the
/// diagonals. [`heading_of`] is its inverse.
pub fn facing_towards(from: Vec2, to: Vec2) -> Angle {
    let dx = i64::from(to.x.raw) - i64::from(from.x.raw);
    let dy = i64::from(to.y.raw) - i64::from(from.y.raw);
    if dx == 0 && dy == 0 {
        return Angle { brads: 0 };
    }
    let (adx, ady) = (dx.abs(), dy.abs());
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
/// Direction only: its length is one octant span of world units, never zero.
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

/// The shortest signed rotation from one facing to another, in brads.
pub fn angle_delta(from: Angle, to: Angle) -> i32 {
    let d = (i32::from(to.brads) - i32::from(from.brads)) & 0xFFFF;
    if d > 32768 { d - 65536 } else { d }
}

/// How far one facing is from another, in brads, ignoring direction.
pub fn facing_gap(one: Angle, other: Angle) -> u16 {
    angle_delta(one, other).unsigned_abs() as u16
}

/// The spot `distance` world units from `from` along the line towards
/// `towards`. `from` itself when the two stand on the same spot.
pub fn point_along(from: Vec2, towards: Vec2, distance: i32) -> Vec2 {
    let dx = i64::from(towards.x.raw) - i64::from(from.x.raw);
    let dy = i64::from(towards.y.raw) - i64::from(from.y.raw);
    let reach = isqrt(dx * dx + dy * dy);
    if reach == 0 {
        return from;
    }
    let step = i64::from(Fixed::from_int(distance).raw);
    Vec2 {
        x: Fixed {
            raw: from.x.raw.saturating_add((dx * step / reach) as i32),
        },
        y: Fixed {
            raw: from.y.raw.saturating_add((dy * step / reach) as i32),
        },
    }
}

/// The spot `distance` world units ahead of a body, along the way it looks.
pub fn spot_ahead(unit: &UnitView, distance: i32) -> Vec2 {
    point_along(unit.pos, unit.pos + heading_of(unit.facing), distance)
}

/// Whether a body stands within `radius` of a spot.
pub fn within(unit: &UnitView, spot: Vec2, radius: i32) -> bool {
    unit.pos.within(spot, Fixed::from_int(radius))
}

/// What a blow is worth after the target's armor and resistance.
pub fn after_mitigation(amount: i32, kind: DamageKind, target: &UnitView) -> i32 {
    match kind {
        DamageKind::Physical => {
            let armor = target.armor.max(Fixed::ZERO);
            let whole = i64::from(Fixed::ONE.raw);
            let den = 100 * whole + i64::from(crate::ARMOR_SCALE) * i64::from(armor.raw);
            (i64::from(amount) * 100 * whole / den) as i32
        }
        DamageKind::Magical => {
            let kept = Fixed::ONE - target.magic_resist.clamp(Fixed::ZERO, Fixed::ONE);
            (Fixed::from_int(amount) * kept).to_int()
        }
        DamageKind::Pure => amount,
    }
}
