//! Deterministic integer geometry: stepping, blocking, turning, bounds.

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

/// The walkability grid of the map.
///
/// One bit per cell, `true` is walkable. The map is open ground; structures
/// close the cells they stand on when the world is built.
#[derive(Clone, Debug)]
pub struct PassGrid {
    /// One bit per cell, row-major.
    bits: Vec<u64>,
}

impl PassGrid {
    /// A fully walkable map.
    pub fn open() -> PassGrid {
        PassGrid {
            bits: vec![u64::MAX; rules::GRID_CELLS * rules::GRID_CELLS / 64],
        }
    }

    /// The cell a position falls into, if it is on the map.
    pub fn cell_of(pos: Vec2) -> Option<(usize, usize)> {
        if pos.x.raw < 0 || pos.y.raw < 0 {
            return None;
        }
        let cx = pos.x.to_int() / rules::GRID_CELL_SIZE;
        let cy = pos.y.to_int() / rules::GRID_CELL_SIZE;
        if cx >= rules::GRID_CELLS as i32 || cy >= rules::GRID_CELLS as i32 {
            return None;
        }
        Some((cx as usize, cy as usize))
    }

    /// The center of a cell.
    pub fn cell_center(cell: (usize, usize)) -> Vec2 {
        Vec2::from_ints(
            cell.0 as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
            cell.1 as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
        )
    }

    /// Whether a cell is walkable.
    pub fn cell_open(&self, cx: usize, cy: usize) -> bool {
        let idx = cy * rules::GRID_CELLS + cx;
        self.bits[idx / 64] & (1 << (idx % 64)) != 0
    }

    /// Whether a position is on the map and walkable.
    pub fn walkable(&self, pos: Vec2) -> bool {
        match PassGrid::cell_of(pos) {
            None => false,
            Some((cx, cy)) => self.cell_open(cx, cy),
        }
    }

    /// Closes every cell whose center lies within `radius` of `center`.
    pub fn block_circle(&mut self, center: Vec2, radius: Fixed) {
        self.paint_circle(center, radius, false);
    }

    /// Reopens every cell whose center lies within `radius` of `center`.
    pub fn open_circle(&mut self, center: Vec2, radius: Fixed) {
        self.paint_circle(center, radius, true);
    }

    /// Closes one cell.
    pub fn close_cell(&mut self, cx: usize, cy: usize) {
        let idx = cy * rules::GRID_CELLS + cx;
        self.bits[idx / 64] &= !(1 << (idx % 64));
    }

    fn paint_circle(&mut self, center: Vec2, radius: Fixed, open: bool) {
        let cells = rules::GRID_CELLS as i32;
        let span = radius.to_int() / rules::GRID_CELL_SIZE + 1;
        let ccx = center.x.to_int() / rules::GRID_CELL_SIZE;
        let ccy = center.y.to_int() / rules::GRID_CELL_SIZE;
        for cy in (ccy - span).max(0)..=(ccy + span).min(cells - 1) {
            for cx in (ccx - span).max(0)..=(ccx + span).min(cells - 1) {
                let c = PassGrid::cell_center((cx as usize, cy as usize));
                if c.within(center, radius) {
                    let idx = cy as usize * rules::GRID_CELLS + cx as usize;
                    if open {
                        self.bits[idx / 64] |= 1 << (idx % 64);
                    } else {
                        self.bits[idx / 64] &= !(1 << (idx % 64));
                    }
                }
            }
        }
    }
}

/// The grid clearance a structure blocks: its own radius, the widest walker
/// and a margin.
pub fn structure_clearance(radius: Fixed) -> Fixed {
    radius + rules::units(rules::HERO_RADIUS + rules::STEER_MARGIN)
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
