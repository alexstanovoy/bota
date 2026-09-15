//! Walking with other bodies in the way: who stands where, whether a step
//! runs into anybody, and easing apart what overlaps.

use bota_proto::{Fixed, Vec2};

use crate::game::{Body, Entity, World};
use crate::game::{clamp_to_map, isqrt64, rules};

impl World {
    /// Lays the body index out from where everything with a hull stands.
    pub fn lay_bodies(&mut self) {
        let mut bodies = std::mem::take(&mut self.body_scratch);
        bodies.clear();
        for entity in self.entities.iter() {
            let (Some(hull), Some(at)) = (self.hull.get(entity), self.transform.get(entity)) else {
                continue;
            };
            let motion = self.motion.get(entity).copied().unwrap_or_default();
            let fixed = self
                .stats
                .get(entity)
                .is_none_or(|stats| stats.move_speed == Fixed::ZERO);
            bodies.push(Body {
                entity,
                at: at.pos,
                radius: hull.collision,
                delta: motion.delta,
                still: motion.still,
                fixed,
            });
        }
        self.bodies.lay(&bodies);
        self.body_scratch = bodies;
    }

    /// The body a step from one spot to another would walk into, the
    /// nearest to where the step begins when there are several.
    ///
    /// A step deeper into any hull is refused; a step out of an overlap is
    /// allowed, so nothing can wedge for good.
    pub fn body_in_the_way(&self, mover: Entity, from: Vec2, next: Vec2) -> Option<Entity> {
        let mine = self.hull.get(mover)?.collision;
        let mut best: Option<(i64, Entity)> = None;
        self.bodies.near(next, mine, |body| {
            if body.entity == mover {
                return;
            }
            let Some(at) = self.transform.get(body.entity).map(|t| t.pos) else {
                return;
            };
            let least = (mine + body.radius).squared_raw();
            let ahead = next.distance_squared(at);
            if ahead >= least || ahead >= from.distance_squared(at) {
                return;
            }
            let near = from.distance_squared(at);
            if best.is_none_or(|(had, _)| near < had) {
                best = Some((near, body.entity));
            }
        });
        best.map(|(_, body)| body)
    }

    /// Whether an entity walks through the bodies in its way.
    pub fn phased(&self, entity: Entity) -> bool {
        self.stats.get(entity).is_some_and(|stats| stats.phased)
    }

    /// Eases apart every pair of bodies whose hulls overlap.
    ///
    /// A body moves at most [`rules::SEPARATION_STEP`] in a tick and never
    /// onto closed ground. A building never moves: the whole correction
    /// falls on whatever walked into it.
    pub fn push_apart(&mut self) {
        let mut bodies = std::mem::take(&mut self.body_scratch);
        bodies.clear();
        for entity in self.entities.iter() {
            let (Some(at), Some(hull)) = (self.transform.get(entity), self.hull.get(entity)) else {
                continue;
            };
            let fixed = self
                .stats
                .get(entity)
                .is_none_or(|stats| stats.move_speed == Fixed::ZERO);
            bodies.push(Body {
                entity,
                at: at.pos,
                radius: hull.collision,
                delta: Vec2::ZERO,
                still: 0,
                fixed,
            });
        }
        let cap = i64::from(rules::units(rules::SEPARATION_STEP).raw);
        let mut push = vec![(0i64, 0i64); bodies.len()];
        for (i, one) in bodies.iter().enumerate() {
            if one.fixed {
                continue;
            }
            if self.phased(one.entity) {
                continue;
            }
            self.bodies.near(one.at, one.radius, |other| {
                if other.entity == one.entity || self.phased(other.entity) {
                    return;
                }
                let Some(there) = self.transform.get(other.entity).map(|t| t.pos) else {
                    return;
                };
                let dx = i64::from(there.x.raw) - i64::from(one.at.x.raw);
                let dy = i64::from(there.y.raw) - i64::from(one.at.y.raw);
                let least = i64::from((one.radius + other.radius).raw);
                let apart = dx * dx + dy * dy;
                if apart >= least * least {
                    return;
                }
                let far = isqrt64(apart);
                // Two on one spot part along the x axis, the earlier entity
                // westward.
                let (ux, uy, len) = if far == 0 {
                    (if one.entity < other.entity { 1 } else { -1 }, 0, 1)
                } else {
                    (dx, dy, far)
                };
                let gap = least - far;
                // A fixed body takes none of it; two that walk share it.
                let mine = if other.fixed { gap } else { gap / 2 };
                push[i].0 -= ux * mine.min(cap) / len;
                push[i].1 -= uy * mine.min(cap) / len;
            });
        }
        for (index, body) in bodies.iter().enumerate() {
            let (mut dx, mut dy) = push[index];
            if body.fixed || (dx == 0 && dy == 0) {
                continue;
            }
            let len = isqrt64(dx * dx + dy * dy);
            if len > cap {
                dx = dx * cap / len;
                dy = dy * cap / len;
            }
            let next = clamp_to_map(Vec2 {
                x: Fixed {
                    raw: body.at.x.raw.saturating_add(dx as i32),
                },
                y: Fixed {
                    raw: body.at.y.raw.saturating_add(dy as i32),
                },
            });
            if !self.clearance.walkable(next) && self.clearance.walkable(body.at) {
                continue;
            }
            if let Some(transform) = self.transform.get_mut(body.entity) {
                transform.pos = next;
            }
        }
        self.body_scratch = bodies;
    }
}
