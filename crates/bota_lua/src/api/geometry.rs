//! Bounded circle queries over visible protocol positions.

#![allow(
    clippy::float_arithmetic,
    reason = "circle intersection candidates are rounded to protocol coordinates before evaluation"
)]

use bota_proto::{Fixed, Vec2};
use luagee::LuaError;

pub(super) fn project(
    position: Vec2,
    velocity: Option<bota_client_core::VelocityEstimate>,
    delay: f64,
) -> Option<Vec2> {
    let Some(velocity) = velocity else {
        return Some(position);
    };
    if velocity.elapsed_ticks == 0 {
        return None;
    }
    let ratio = delay * 30.0 / f64::from(velocity.elapsed_ticks);
    let origin = point(position);
    let delta = point(velocity.delta);
    fixed([origin[0] + delta[0] * ratio, origin[1] + delta[1] * ratio])
}

type Point = [f64; 2];

fn point(value: Vec2) -> Point {
    [
        f64::from(value.x.raw) / 65536.0,
        f64::from(value.y.raw) / 65536.0,
    ]
}

fn fixed(value: Point) -> Option<Vec2> {
    let component = |number: f64| {
        let raw = (number * 65536.0).round_ties_even();
        (raw.is_finite() && raw >= f64::from(i32::MIN) && raw <= f64::from(i32::MAX))
            .then_some(Fixed { raw: raw as i32 })
    };
    Some(Vec2 {
        x: component(value[0])?,
        y: component(value[1])?,
    })
}

fn squared(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

fn intersections(a: Point, ar: f64, b: Point, br: f64) -> Option<[Point; 2]> {
    let distance = squared(a, b).sqrt();
    if distance == 0.0 || distance > ar + br || distance < (ar - br).abs() {
        return None;
    }
    let along = (ar * ar - br * br + distance * distance) / (2.0 * distance);
    let height = (ar * ar - along * along).max(0.0).sqrt();
    let direction = [(b[0] - a[0]) / distance, (b[1] - a[1]) / distance];
    let middle = [a[0] + along * direction[0], a[1] + along * direction[1]];
    Some([
        [
            middle[0] - height * direction[1],
            middle[1] + height * direction[0],
        ],
        [
            middle[0] + height * direction[1],
            middle[1] - height * direction[0],
        ],
    ])
}

struct CircleSearch<'a> {
    origin: Point,
    range: f64,
    radius: f64,
    units: &'a [Vec2],
    best: (Vec2, usize),
}

impl CircleSearch<'_> {
    fn consider(&mut self, candidate: Point,
        charge: &mut impl FnMut(u64) -> Result<(), LuaError>) -> Result<(), LuaError> {
        charge(1)?;
        let Some(candidate) = fixed(candidate) else { return Ok(()); };
        let candidate_point = point(candidate);
        if squared(self.origin, candidate_point) > self.range * self.range { return Ok(()); }
        // Only reachable candidates scan targets, and that scan is paid before it runs.
        charge(self.units.len() as u64)?;
        let count = self.units.iter().filter(|&&unit| squared(point(unit), candidate_point) <= self.radius * self.radius).count();
        if count > self.best.1 || (count == self.best.1 && (candidate.x.raw, candidate.y.raw) < (self.best.0.x.raw, self.best.0.y.raw)) {
            self.best = (candidate, count);
        }
        Ok(())
    }
}

pub(super) fn best_circle(base: Vec2, range: f64, radius: f64, units: &[Vec2],
    charge: &mut impl FnMut(u64) -> Result<(), LuaError>) -> Result<(Vec2, usize), LuaError> {
    let origin = point(base);
    let mut search = CircleSearch { origin, range, radius, units, best: (base, 0) };
    search.consider(origin, charge)?;
    for (index, &unit) in units.iter().enumerate() {
        // Fixed-size intersection arithmetic is independent of the following containment scans.
        charge(8)?;
        let center = point(unit);
        search.consider(center, charge)?;
        let distance = squared(origin, center).sqrt();
        if distance > 0.0 {
            let ratio = (range / distance).min(1.0);
            search.consider([origin[0] + (center[0] - origin[0]) * ratio,
                origin[1] + (center[1] - origin[1]) * ratio], charge)?;
        }
        if let Some(points) = intersections(origin, range, center, radius) {
            for point in points { search.consider(point, charge)?; }
        }
        for &other in &units[..index] {
            charge(8)?;
            if let Some(points) = intersections(center, radius, point(other), radius) {
                for point in points { search.consider(point, charge)?; }
            }
        }
    }
    Ok(search.best)
}
