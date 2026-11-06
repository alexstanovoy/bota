//! Bounded circle queries over visible protocol positions.

#![allow(clippy::float_arithmetic, reason = "circle intersection candidates are rounded to protocol coordinates before evaluation")]

use bota_proto::{Fixed, Vec2};

type Point = [f64; 2];

fn point(value: Vec2) -> Point { [f64::from(value.x.raw) / 65536.0, f64::from(value.y.raw) / 65536.0] }

fn fixed(value: Point) -> Option<Vec2> {
    let component = |number: f64| {
        let raw = (number * 65536.0).round_ties_even();
        (raw.is_finite() && raw >= f64::from(i32::MIN) && raw <= f64::from(i32::MAX)).then_some(Fixed { raw: raw as i32 })
    };
    Some(Vec2 { x: component(value[0])?, y: component(value[1])? })
}

fn squared(a: Point, b: Point) -> f64 { (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) }

fn intersections(a: Point, ar: f64, b: Point, br: f64) -> Option<[Point; 2]> {
    let distance = squared(a, b).sqrt();
    if distance == 0.0 || distance > ar + br || distance < (ar - br).abs() { return None; }
    let along = (ar * ar - br * br + distance * distance) / (2.0 * distance);
    let height = (ar * ar - along * along).max(0.0).sqrt();
    let direction = [(b[0] - a[0]) / distance, (b[1] - a[1]) / distance];
    let middle = [a[0] + along * direction[0], a[1] + along * direction[1]];
    Some([[middle[0] - height * direction[1], middle[1] + height * direction[0]], [middle[0] + height * direction[1], middle[1] - height * direction[0]]])
}

pub(super) fn best_circle(base: Vec2, range: f64, radius: f64, units: &[Vec2]) -> (Vec2, usize) {
    let origin = point(base);
    let mut best = (base, 0);
    let mut consider = |candidate: Point| {
        let Some(candidate) = fixed(candidate) else { return };
        let candidate_point = point(candidate);
        if squared(origin, candidate_point) > range * range { return; }
        let count = units.iter().filter(|&&unit| squared(point(unit), candidate_point) <= radius * radius).count();
        if count > best.1 || (count == best.1 && (candidate.x.raw, candidate.y.raw) < (best.0.x.raw, best.0.y.raw)) { best = (candidate, count); }
    };
    consider(origin);
    for (index, &unit) in units.iter().enumerate() {
        let center = point(unit);
        consider(center);
        let distance = squared(origin, center).sqrt();
        if distance > 0.0 {
            let ratio = (range / distance).min(1.0);
            consider([origin[0] + (center[0] - origin[0]) * ratio, origin[1] + (center[1] - origin[1]) * ratio]);
        }
        if let Some(points) = intersections(origin, range, center, radius) { for point in points { consider(point); } }
        for &other in &units[..index] {
            if let Some(points) = intersections(center, radius, point(other), radius) { for point in points { consider(point); } }
        }
    }
    best
}
