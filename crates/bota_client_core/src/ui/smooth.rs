//! Motion between snapshots: where things are drawn on frames that fall
//! between two ticks.
//!
//! Every unit and missile is drawn on the line from where the snapshot
//! before the latest put it to where the latest puts it, so what is shown
//! trails the server by at most one tick and never jumps.

use bota_proto::{Angle, EntityId, ProjectileView, UnitView, WorldView};

/// Seconds between snapshots assumed before any have been timed.
const FIRST_INTERVAL: f32 = 1.0 / 30.0;
/// A move longer than this in one tick, in world units, is a jump: a blink,
/// a teleport or a respawn. It is drawn at once.
const JUMP: f32 = 450.0;

/// The clock between snapshots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smooth {
    /// Seconds since the latest snapshot came in.
    pub since: f32,
    /// Seconds between snapshots, a running average.
    pub interval: f32,
}

impl Smooth {
    /// A clock that has timed nothing yet.
    pub fn new() -> Smooth {
        Smooth {
            since: 0.0,
            interval: FIRST_INTERVAL,
        }
    }

    /// Notes that a snapshot came in.
    ///
    /// Snapshots that come in together, as after a stall or in a fast
    /// replay, leave the average alone.
    pub fn snapshot(&mut self) {
        if self.since > 0.002 {
            let seen = self.since.clamp(1.0 / 480.0, 0.25);
            self.interval += (seen - self.interval) * 0.2;
        }
        self.since = 0.0;
    }

    /// Moves the clock on by one frame.
    pub fn advance(&mut self, dt: f32) {
        self.since = (self.since + dt.max(0.0)).min(10.0);
    }

    /// How far between the two latest snapshots the frame is, 0 to 1.
    pub fn alpha(&self) -> f32 {
        (self.since / self.interval.max(1e-3)).clamp(0.0, 1.0)
    }
}

impl Default for Smooth {
    fn default() -> Smooth {
        Smooth::new()
    }
}

/// A unit as drawn this frame.
#[derive(Clone, Copy, Debug)]
pub struct Body<'a> {
    /// What the latest snapshot says of it.
    pub unit: &'a UnitView,
    /// Drawn world X.
    pub x: f32,
    /// Drawn world Y.
    pub y: f32,
    /// Drawn facing, radians counter-clockwise from the world X axis.
    pub facing: f32,
}

/// A missile as drawn this frame, and where it was a tick ago.
#[derive(Clone, Copy, Debug)]
pub struct Flight {
    /// What the latest snapshot says of it.
    pub missile: ProjectileView,
    /// Drawn world X.
    pub x: f32,
    /// Drawn world Y.
    pub y: f32,
    /// World X one tick back along its path; its own X when just launched.
    pub tail_x: f32,
    /// World Y one tick back along its path.
    pub tail_y: f32,
}

/// Radians of a wire angle.
pub fn radians(angle: Angle) -> f32 {
    f32::from(angle.brads) / 65536.0 * std::f32::consts::TAU
}

/// The angle `k` of the way from `a` to `b` by the shorter turn.
fn turn(a: Angle, b: Angle, k: f32) -> f32 {
    let delta = b.brads.wrapping_sub(a.brads) as i16;
    radians(a) + f32::from(delta) / 65536.0 * std::f32::consts::TAU * k
}

/// Where between `from` and `to` a body is drawn, or `to` across a jump.
fn between(from: (f32, f32), to: (f32, f32), k: f32) -> (f32, f32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    if dx * dx + dy * dy > JUMP * JUMP {
        return to;
    }
    (from.0 + dx * k, from.1 + dy * k)
}

/// Every unit of `view` as drawn `k` of the way from `prev`, in the order of
/// the view: sorted by id.
pub fn bodies<'a>(view: &'a WorldView, prev: Option<&WorldView>, k: f32) -> Vec<Body<'a>> {
    view.units
        .iter()
        .map(|unit| {
            let to = (unit.pos.x.to_f32(), unit.pos.y.to_f32());
            let before = prev.and_then(|prev| {
                prev.units
                    .binary_search_by_key(&unit.id, |u| u.id)
                    .ok()
                    .map(|at| &prev.units[at])
            });
            match before {
                Some(old) => {
                    let from = (old.pos.x.to_f32(), old.pos.y.to_f32());
                    let (x, y) = between(from, to, k);
                    Body {
                        unit,
                        x,
                        y,
                        facing: turn(old.facing, unit.facing, k),
                    }
                }
                None => Body {
                    unit,
                    x: to.0,
                    y: to.1,
                    facing: radians(unit.facing),
                },
            }
        })
        .collect()
}

/// The body of one unit among `bodies`, which are sorted by id.
pub fn body_of<'b, 'a>(bodies: &'b [Body<'a>], id: EntityId) -> Option<&'b Body<'a>> {
    bodies
        .binary_search_by_key(&id, |body| body.unit.id)
        .ok()
        .map(|at| &bodies[at])
}

/// Every missile of `view` as drawn `k` of the way from `prev`.
pub fn flights(view: &WorldView, prev: Option<&WorldView>, k: f32) -> Vec<Flight> {
    view.projectiles
        .iter()
        .map(|missile| {
            let to = (missile.pos.x.to_f32(), missile.pos.y.to_f32());
            let from = prev
                .and_then(|prev| {
                    prev.projectiles
                        .binary_search_by_key(&missile.id, |p| p.id)
                        .ok()
                        .map(|at| &prev.projectiles[at])
                })
                .map_or(to, |old| (old.pos.x.to_f32(), old.pos.y.to_f32()));
            let (x, y) = between(from, to, k);
            Flight {
                missile: *missile,
                x,
                y,
                tail_x: x - (to.0 - from.0),
                tail_y: y - (to.1 - from.1),
            }
        })
        .collect()
}
