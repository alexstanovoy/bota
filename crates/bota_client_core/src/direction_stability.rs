//! Direction changes over one fully observed, consecutive window.
#![allow(
    clippy::float_arithmetic,
    reason = "the directional statistic is defined in single-precision arithmetic; it is not a simulation rule"
)]
use bota_proto::Vec2;
/// Enough positions for this simulation's one-second window, without retaining unused history.
pub const DIRECTION_HISTORY_CAPACITY: usize = bota_core::rules::TICKS_PER_SECOND as usize + 2;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectionHistory {
    points: [Vec2; DIRECTION_HISTORY_CAPACITY],
    next: usize,
    len: usize,
    last_tick: Option<u32>,
}
impl Default for DirectionHistory {
    fn default() -> Self {
        Self {
            points: [Vec2::ZERO; DIRECTION_HISTORY_CAPACITY],
            next: 0,
            len: 0,
            last_tick: None,
        }
    }
}
impl DirectionHistory {
    /// O(1), fixed storage. No interpolation, zero padding or teleport-specific reset.
    pub fn observe(&mut self, tick: u32, point: Vec2) {
        if self
            .last_tick
            .is_some_and(|last| last.checked_add(1) != Some(tick))
        {
            *self = Self::default();
        }
        self.points[self.next] = point;
        self.next = (self.next + 1) % DIRECTION_HISTORY_CAPACITY;
        self.len = (self.len + 1).min(DIRECTION_HISTORY_CAPACITY);
        self.last_tick = Some(tick);
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    /// Number of positions required. An unsupported cadence is unavailable, never resampled.
    pub fn window_positions(tick_rate: u16) -> Option<usize> {
        if tick_rate == 0 {
            return None;
        }
        let dt = 1.0f32 / f32::from(tick_rate);
        let pairs = (1.0f32 / dt) as usize;
        let count = pairs.min(118) + 2;
        (count <= DIRECTION_HISTORY_CAPACITY).then_some(count)
    }
    /// O(window), at most DIRECTION_HISTORY_CAPACITY-2 adjacent direction pairs.
    /// Fuel consumers must pay for the whole window before this query. Missing samples are None.
    pub fn stability(&self, tick_rate: u16) -> Option<f32> {
        let count = Self::window_positions(tick_rate)?;
        if self.len < count {
            return None;
        }
        let start = (self.next + DIRECTION_HISTORY_CAPACITY - count) % DIRECTION_HISTORY_CAPACITY;
        let point = |at: usize| self.points[(start + at) % DIRECTION_HISTORY_CAPACITY];
        let mut previous = direction(point(0), point(1));
        let mut sum = 0.0f32;
        for at in 1..count - 1 {
            let current = direction(point(at), point(at + 1));
            let a = zero(previous);
            let b = zero(current);
            if a != b {
                sum += 1.0;
            } else if !a {
                sum += 1.0 - (current.0 * previous.0 + current.1 * previous.1);
            }
            previous = current;
        }
        Some(1.0 - (sum * 10.0 / (count - 2) as f32).clamp(0.0, 1.0))
    }
}
fn position(point: Vec2) -> (f32, f32) {
    let offset = i64::from(bota_core::rules::MAP_SIZE / 2) * 65536;
    (
        ((i64::from(point.x.raw) - offset) as f32) / 65536.0,
        ((i64::from(point.y.raw) - offset) as f32) / 65536.0,
    )
}
fn direction(a: Vec2, b: Vec2) -> (f32, f32) {
    let a = position(a);
    let b = position(b);
    let x = b.0 - a.0;
    let y = b.1 - a.1;
    let length = (x * x + 0.0 + y * y).sqrt();
    if length == 0.0 {
        (0.0, 0.0)
    } else {
        let inverse = 1.0 / length;
        (x * inverse, y * inverse)
    }
}
fn zero(value: (f32, f32)) -> bool {
    value.0 * value.0 + value.1 * value.1 < 0.001
}
#[cfg(test)]
mod tests {
    use super::*;
    fn result(points: impl Iterator<Item = Vec2>) -> f32 {
        let mut history = DirectionHistory::default();
        for (tick, point) in points.enumerate() {
            history.observe(tick as u32, point);
        }
        history.stability(30).unwrap()
    }
    #[test]
    fn analytic_straight_stationary_reversal_and_right_angle_windows() {
        assert_eq!(DirectionHistory::window_positions(30), Some(31));
        assert_eq!(
            result((0..31).map(|tick| Vec2::from_ints(9000 + tick * 3, 9000 + tick * 4))),
            1.0
        );
        assert_eq!(result((0..31).map(|_| Vec2::from_ints(9000, 9000))), 1.0);
        assert_eq!(
            result((0..31).map(|tick| Vec2::from_ints(9000 + tick % 2, 9000))),
            0.0
        );
        let corner = result((0..31).map(|tick| {
            if tick <= 15 {
                Vec2::from_ints(9000 + tick, 9000)
            } else {
                Vec2::from_ints(9015, 9000 + tick - 15)
            }
        }));
        assert!(
            (corner - 0.6551724).abs() < 0.000001,
            "one right angle contributes exactly one to the 29-pair sum: {corner}"
        );
        let diagonal = result((0..31).map(|tick| {
            if tick <= 15 {
                Vec2::from_ints(9000 + tick, 9000)
            } else {
                Vec2::from_ints(9000 + tick, 9000 + tick - 15)
            }
        }));
        assert!((diagonal - 0.8990023).abs() < 0.000001);
        let teleport = result(
            (0..31)
                .map(|tick| Vec2::from_ints(9000 + tick + if tick > 15 { 1000 } else { 0 }, 9000)),
        );
        assert_eq!(teleport, 1.0);
        let stopped = result((0..31).map(|tick| Vec2::from_ints(9000 + tick.min(15), 9000)));
        assert!((stopped - 0.6551724).abs() < 0.000001);
    }
    #[test]
    fn a_gap_clears_observations_and_ring_wraps_without_changing_the_window() {
        let mut history = DirectionHistory::default();
        for tick in 0..200 {
            history.observe(tick, Vec2::from_ints(9000 + tick as i32, 9000));
        }
        assert_eq!(history.stability(30), Some(1.0));
        history.observe(202, Vec2::from_ints(9202, 9000));
        assert_eq!(history.stability(30), None);
        for tick in 203..232 {
            history.observe(tick, Vec2::from_ints(9000 + tick as i32, 9000));
        }
        assert_eq!(history.stability(30), None);
        history.observe(232, Vec2::from_ints(9232, 9000));
        assert_eq!(history.stability(30), Some(1.0));
        history.clear();
        assert_eq!(history.stability(30), None);
    }
}
