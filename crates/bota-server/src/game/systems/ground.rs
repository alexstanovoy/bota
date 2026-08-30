//! Walking with the ground and other bodies in the way.
//!
//! Two ways of getting past something, kept apart. A creep marches: it aims at
//! the next point of its route until the step ahead is shut, then picks one
//! side and works round. Anything a player drives walks: it slides along
//! whatever it grazes, keeping the part of its step that survives.

use bota_proto::{Fixed, Vec2};

use crate::game::{Entity, TraceSide, World};
use crate::game::{clamp_to_map, isqrt64, move_towards, rules};

impl World {
    /// Whether stepping from one spot to another runs an entity into a body.
    ///
    /// A step deeper into any hull is refused; a step out of an overlap is
    /// allowed, so nothing can wedge for good.
    pub fn blocked_by_bodies(&self, mover: Entity, from: Vec2, next: Vec2) -> bool {
        let Some(mine) = self.hull.get(mover).map(|h| h.radius) else {
            return false;
        };
        for other in self.entities.iter() {
            if other == mover {
                continue;
            }
            let (Some(theirs), Some(at)) = (
                self.hull.get(other).map(|h| h.radius),
                self.transform.get(other).map(|t| t.pos),
            ) else {
                continue;
            };
            let least = i64::from((mine + theirs).raw);
            let ahead = next.distance_squared(at);
            if ahead < least * least && ahead < from.distance_squared(at) {
                return true;
            }
        }
        false
    }

    /// Whether a spot is one the mover may take: open ground, no hull.
    ///
    /// An entity already standing in a closed cell may always leave it.
    fn step_free(&self, mover: Entity, from: Vec2, next: Vec2, shoving: bool) -> bool {
        // What flies is over all of it: closed ground stops only what walks.
        let flies = self.stats.get(mover).is_some_and(|stats| stats.flies);
        if !flies && !self.grid.walkable(next) && self.grid.walkable(from) {
            return false;
        }
        shoving || self.phased(mover) || !self.blocked_by_bodies(mover, from, next)
    }

    /// Whether an entity walks through the bodies in its way.
    pub fn phased(&self, entity: Entity) -> bool {
        self.stats.get(entity).is_some_and(|stats| stats.phased)
    }

    /// One step of a march, refusing anything shut.
    ///
    /// A creep that has stood still for [`rules::MARCH_SHOVE_TICKS`] shoves
    /// instead: it walks into the bodies in its way and is eased out of them
    /// after. Closed ground stops it either way.
    pub fn march_step(&self, mover: Entity, aim: Vec2, step: Fixed) -> Vec2 {
        let Some(from) = self.transform.get(mover).map(|t| t.pos) else {
            return aim;
        };
        let shoving = self
            .march
            .get(mover)
            .is_some_and(|march| march.shove >= rules::MARCH_SHOVE_TICKS);
        let next = clamp_to_map(move_towards(from, aim, step));
        if self.step_free(mover, from, next, shoving) {
            next
        } else {
            from
        }
    }

    /// Where a creep aims: its waypoint, or a way round the body in the way.
    ///
    /// The side is settled on first touch and kept while the way ahead is
    /// still shut. Within that side the aim swings an eighth of a turn off the
    /// line, then a quarter, then three eighths; only with the whole side
    /// exhausted is the other side tried, and the last resort is straight
    /// back. A side is worked all the way through before the other is
    /// touched: alternating between them wiggles at a wall for ever.
    pub fn march_aim(
        &self,
        mover: Entity,
        waypoint: Vec2,
        step: Fixed,
        holding: Option<TraceSide>,
    ) -> (Vec2, Option<TraceSide>) {
        let Some(from) = self.transform.get(mover).map(|t| t.pos) else {
            return (waypoint, None);
        };
        let straight = clamp_to_map(move_towards(from, waypoint, step));
        if self.step_free(mover, from, straight, false) {
            return (waypoint, None);
        }
        let first = holding.unwrap_or_else(|| {
            self.blocking_body(mover, straight)
                .map_or(TraceSide::Left, |body| pick_side(from, body, waypoint))
        });
        let reach = i64::from(
            (self.hull.get(mover).map_or(Fixed::ZERO, |h| h.radius)
                + rules::units(rules::TRACE_CLEARANCE))
            .raw,
        ) * 2;
        let dx = i64::from(waypoint.x.raw) - i64::from(from.x.raw);
        let dy = i64::from(waypoint.y.raw) - i64::from(from.y.raw);
        for side in [first, other_side(first)] {
            for turn in [Turn::Eighth, Turn::Quarter, Turn::ThreeEighths] {
                let (tx, ty) = turn.apply(dx, dy, sign(side));
                let aim = aim_at(from, tx, ty, reach);
                let ahead = clamp_to_map(move_towards(from, aim, step));
                if ahead != from && self.step_free(mover, from, ahead, false) {
                    return (aim, Some(side));
                }
            }
        }
        (aim_at(from, -dx, -dy, reach), Some(first))
    }

    /// The body a straight step would walk into, nearest first.
    fn blocking_body(&self, mover: Entity, straight: Vec2) -> Option<Vec2> {
        let mine = self.hull.get(mover).map(|h| h.radius)?;
        let from = self.transform.get(mover).map(|t| t.pos)?;
        let mut best: Option<(i64, Vec2)> = None;
        for other in self.entities.iter() {
            if other == mover {
                continue;
            }
            let (Some(theirs), Some(at)) = (
                self.hull.get(other).map(|h| h.radius),
                self.transform.get(other).map(|t| t.pos),
            ) else {
                continue;
            };
            let least = i64::from((mine + theirs).raw);
            if straight.distance_squared(at) >= least * least {
                continue;
            }
            let near = from.distance_squared(at);
            if best.is_none_or(|(had, _)| near < had) {
                best = Some((near, at));
            }
        }
        best.map(|(_, at)| at)
    }

    /// One step of a walk, sliding along whatever it grazes.
    ///
    /// A step square into a body keeps nothing; one that only grazes keeps
    /// nearly all of it.
    pub fn walk_step(&self, mover: Entity, aim: Vec2, step: Fixed) -> Vec2 {
        let Some(from) = self.transform.get(mover).map(|t| t.pos) else {
            return aim;
        };
        let straight = clamp_to_map(move_towards(from, aim, step));
        if self.step_free(mover, from, straight, false) {
            return straight;
        }
        if let Some(body) = self.blocking_body(mover, straight) {
            for side in [1, -1] {
                let slid = tangent_step(from, body, aim, step, side);
                if slid != from && self.step_free(mover, from, slid, false) {
                    return slid;
                }
            }
        }
        // Whatever is in the way is not a body: try shorter steps at the wall.
        for part in 1..=rules::STEP_FIT_TRIES {
            let shorter = Fixed {
                raw: step.raw / (1 << part),
            };
            let closer = clamp_to_map(move_towards(from, aim, shorter));
            if closer != from && self.step_free(mover, from, closer, false) {
                return closer;
            }
        }
        from
    }

    /// Eases apart every pair of bodies whose hulls overlap.
    ///
    /// A body moves at most [`rules::SEPARATION_STEP`] in a tick and never
    /// into a closed cell. A building never moves: the whole correction falls
    /// on whatever walked into it.
    pub fn push_apart(&mut self) {
        let bodies: Vec<(Entity, Vec2, i64, bool)> = self
            .entities
            .iter()
            .filter_map(|entity| {
                let at = self.transform.get(entity)?.pos;
                let radius = self.hull.get(entity)?.radius;
                let still = self
                    .stats
                    .get(entity)
                    .is_none_or(|s| s.move_speed == Fixed::ZERO);
                Some((entity, at, i64::from(radius.raw), still))
            })
            .collect();
        let cap = i64::from(rules::units(rules::SEPARATION_STEP).raw);
        let mut push = vec![(0i64, 0i64); bodies.len()];
        for i in 0..bodies.len() {
            for j in i + 1..bodies.len() {
                let (one, other) = (&bodies[i], &bodies[j]);
                if one.3 && other.3 {
                    continue;
                }
                // Neither side of a pair is eased apart while one of them
                // walks through bodies.
                if self.phased(one.0) || self.phased(other.0) {
                    continue;
                }
                let dx = i64::from(other.1.x.raw) - i64::from(one.1.x.raw);
                let dy = i64::from(other.1.y.raw) - i64::from(one.1.y.raw);
                let least = one.2 + other.2;
                let apart = dx * dx + dy * dy;
                if apart >= least * least {
                    continue;
                }
                let far = isqrt64(apart);
                let (ux, uy, len) = if far == 0 { (1, 0, 1) } else { (dx, dy, far) };
                let gap = least - far;
                let (mine, theirs) = match (one.3, other.3) {
                    (true, _) => (0, gap),
                    (_, true) => (gap, 0),
                    _ => (gap / 2, gap / 2),
                };
                push[i].0 -= ux * mine.min(cap) / len;
                push[i].1 -= uy * mine.min(cap) / len;
                push[j].0 += ux * theirs.min(cap) / len;
                push[j].1 += uy * theirs.min(cap) / len;
            }
        }
        for (index, (entity, at, _, still)) in bodies.iter().enumerate() {
            let (mut dx, mut dy) = push[index];
            if *still || (dx == 0 && dy == 0) {
                continue;
            }
            let len = isqrt64(dx * dx + dy * dy);
            if len > cap {
                dx = dx * cap / len;
                dy = dy * cap / len;
            }
            let next = clamp_to_map(Vec2 {
                x: Fixed {
                    raw: at.x.raw.saturating_add(dx as i32),
                },
                y: Fixed {
                    raw: at.y.raw.saturating_add(dy as i32),
                },
            });
            if !self.grid.walkable(next) && self.grid.walkable(*at) {
                continue;
            }
            if let Some(transform) = self.transform.get_mut(*entity) {
                transform.pos = next;
            }
        }
    }
}

/// How far off the line to the waypoint an aim is swung.
#[derive(Clone, Copy)]
enum Turn {
    /// An eighth of a turn.
    Eighth,
    /// A quarter turn, square across the line.
    Quarter,
    /// Three eighths: past square, still forward of back.
    ThreeEighths,
}

impl Turn {
    /// The direction swung this far off `(dx, dy)`, `sign` picking the way
    /// round. Only the direction is used, not the length.
    fn apply(self, dx: i64, dy: i64, sign: i64) -> (i64, i64) {
        match self {
            Turn::Eighth => (dx - sign * dy, dy + sign * dx),
            Turn::Quarter => (-sign * dy, sign * dx),
            Turn::ThreeEighths => (-sign * dy - dx, sign * dx - dy),
        }
    }
}

/// Which way round a side turns.
fn sign(side: TraceSide) -> i64 {
    match side {
        TraceSide::Left => 1,
        TraceSide::Right => -1,
    }
}

/// The other way round.
fn other_side(side: TraceSide) -> TraceSide {
    match side {
        TraceSide::Left => TraceSide::Right,
        TraceSide::Right => TraceSide::Left,
    }
}

/// Which way round leaves the mover nearer its waypoint. Ties go left.
fn pick_side(from: Vec2, body: Vec2, waypoint: Vec2) -> TraceSide {
    let bx = i64::from(body.x.raw) - i64::from(from.x.raw);
    let by = i64::from(body.y.raw) - i64::from(from.y.raw);
    let wx = i64::from(waypoint.x.raw) - i64::from(from.x.raw);
    let wy = i64::from(waypoint.y.raw) - i64::from(from.y.raw);
    if bx * wy - by * wx < 0 {
        TraceSide::Right
    } else {
        TraceSide::Left
    }
}

/// A point `reach` away from `from` along a direction.
fn aim_at(from: Vec2, dx: i64, dy: i64, reach: i64) -> Vec2 {
    let len = isqrt64(dx * dx + dy * dy);
    if len == 0 {
        return from;
    }
    clamp_to_map(Vec2 {
        x: Fixed {
            raw: from.x.raw.saturating_add((dx * reach / len) as i32),
        },
        y: Fixed {
            raw: from.y.raw.saturating_add((dy * reach / len) as i32),
        },
    })
}

/// The part of a step that survives sliding along the body in the way.
fn tangent_step(from: Vec2, body: Vec2, aim: Vec2, step: Fixed, side: i64) -> Vec2 {
    let nx = i64::from(body.x.raw) - i64::from(from.x.raw);
    let ny = i64::from(body.y.raw) - i64::from(from.y.raw);
    let n_len = isqrt64(nx * nx + ny * ny);
    if n_len == 0 {
        return from;
    }
    let (tx, ty) = (-ny * side, nx * side);
    let dx = i64::from(aim.x.raw) - i64::from(from.x.raw);
    let dy = i64::from(aim.y.raw) - i64::from(from.y.raw);
    let d_len = isqrt64(dx * dx + dy * dy);
    if d_len == 0 {
        return from;
    }
    // How much of the wanted direction lies along the tangent. The lengths
    // divide last: dividing first rounds the whole thing to nothing.
    let along = (i128::from(dx) * i128::from(tx) + i128::from(dy) * i128::from(ty))
        / (i128::from(n_len) * i128::from(d_len));
    if along < 0 {
        return from;
    }
    let floor = i64::from(step.raw) / i64::from(rules::SLIDE_FLOOR_PART);
    let kept = ((i128::from(step.raw) * along) as i64).max(floor);
    clamp_to_map(Vec2 {
        x: Fixed {
            raw: from.x.raw.saturating_add((tx * kept / n_len) as i32),
        },
        y: Fixed {
            raw: from.y.raw.saturating_add((ty * kept / n_len) as i32),
        },
    })
}
