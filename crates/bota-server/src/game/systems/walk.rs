//! Walking: turning towards where an entity is going, then taking its step.

use bota_proto::{Fixed, Vec2};

use crate::game::{Entity, Route, UnitOrder, World};
use crate::game::{facing_gap, facing_towards, find_path, grid_los, per_tick, rules, turn_towards};

impl World {
    /// Turns and steps everything that has somewhere to be.
    ///
    /// Being held roots outright, and so does a channel; a swing roots
    /// whoever is making it: from
    /// the moment it begins until the recovery after it runs out, the entity
    /// does not leave the spot it stands on, though it keeps coming round to
    /// what it is swinging at.
    ///
    /// Past that, what an entity is set on decides first: with its target in
    /// reach it stands still and comes round to it, out of reach it walks at
    /// it. Only with nothing to fight does it walk where it was told.
    ///
    /// Turning comes first and costs the tick: an entity more than
    /// [`rules::TURN_TOLERANCE_BRADS`] off the way it wants to face stands
    /// still until it has come round. A creep marches round what is in its
    /// way; anything a player drives slides along it.
    pub fn walk_bodies(&mut self) {
        for entity in self.entities.iter().collect::<Vec<_>>() {
            // Held or channelling roots outright: there is nothing to come
            // round to.
            if self.held(entity) || self.is_channelling(entity) {
                continue;
            }
            // Mid-swing it comes round to what the swing was begun against;
            // recovering from one, to whatever it is set on now.
            let (rooted, face) = match self.attacking.get(entity).copied() {
                Some(state) if state.windup.is_some() => {
                    (true, state.windup.map(|windup| windup.target))
                }
                Some(state) if state.recovering > 0 => (true, self.target_of(entity)),
                _ => (false, None),
            };
            if rooted {
                if let Some(at) = face.and_then(|on| self.transform.get(on)).map(|t| t.pos) {
                    self.turn_to(entity, at);
                }
                continue;
            }
            // What it is set on comes before where it was told to go: in
            // reach it stands and comes round, out of reach it closes.
            //
            // An attack order at something it cannot strike still walks it
            // there, and right up to it: reach is only worth stopping at when
            // there is something to do from reach.
            // A cast aimed further off than it reaches walks the caster in,
            // and answers before anything else it was told to do.
            if let Some(cast) = self.casting.get(entity).copied()
                && let Some(aim) = self.cast_spot(entity, cast)
            {
                let reach = crate::game::ability_def(self.ability_in(entity, cast.slot))
                    .map_or(0, |def| def.range);
                if reach > 0 {
                    self.walk_at(entity, aim, rules::units(reach));
                    continue;
                }
            }
            let ordered_at = match self.orders.get(entity).map(|o| o.current) {
                Some(UnitOrder::Attack { target, .. } | UnitOrder::Follow { target, .. }) => {
                    Some(target)
                }
                _ => None,
            };
            let chosen = self.target_of(entity).filter(|on| self.alive(*on));
            let ordered = ordered_at.filter(|on| self.alive(*on));
            let on_target = match (chosen, ordered) {
                // Something it may strike: it closes only to its reach and
                // stops there.
                (Some(on), _) => self
                    .transform
                    .get(on)
                    .map(|at| (at.pos, self.in_reach(entity, on))),
                // Something it may not: reach means nothing, so it closes
                // until the bodies touch and stands there, following it for
                // as long as the order stands.
                (None, Some(on)) => self
                    .transform
                    .get(on)
                    .map(|at| (at.pos, self.bodies_touch(entity, on))),
                (None, None) => None,
            };
            let holding = matches!(
                self.orders.get(entity).map(|o| o.current),
                Some(UnitOrder::Hold)
            );
            let (dest, facing_only) = match on_target {
                // Holding, it comes round to what it is set on but never
                // leaves the spot it was left on.
                Some((at, in_reach)) => (at, in_reach || holding),
                None => {
                    let Some(dest) = self
                        .orders
                        .get(entity)
                        .and_then(|o| destination(&o.current))
                    else {
                        continue;
                    };
                    (dest, false)
                }
            };
            let (Some(from), Some(stats)) = (
                self.transform.get(entity).map(|t| t.pos),
                self.stats.get(entity).copied(),
            ) else {
                continue;
            };
            if from == dest {
                continue;
            }
            if facing_only {
                self.turn_to(entity, dest);
                continue;
            }
            let waypoint = self.next_corner(entity, from, dest);
            let step = per_tick(stats.move_speed);
            let marching = self.march.get(entity).is_some();
            let (aim, trace) = if marching {
                let held = self.march.get(entity).and_then(|m| m.trace);
                self.march_aim(entity, waypoint, step, held)
            } else {
                (waypoint, None)
            };
            let wanted = facing_towards(from, aim);
            let facing = turn_towards(
                self.transform.get(entity).expect("looked up above").facing,
                wanted,
                stats.turn_rate,
            );
            let mut next = from;
            if facing_gap(facing, wanted) <= rules::TURN_TOLERANCE_BRADS {
                next = if marching {
                    self.march_step(entity, aim, step)
                } else {
                    self.walk_step(entity, aim, step)
                };
            }
            if let Some(mut march) = self.march.get(entity).copied() {
                march.shove = if next == from {
                    march.shove.saturating_add(1)
                } else {
                    march.shove.saturating_sub(1)
                };
                march.trace = trace;
                self.march.insert(entity, march);
            }
            if let Some(transform) = self.transform.get_mut(entity) {
                transform.facing = facing;
                transform.pos = next;
            }
        }
    }

    /// Whether two bodies stand near enough to be touching.
    ///
    /// A hair further apart than the hulls themselves, so what stops here is
    /// not overlapping and is not eased away again by [`World::push_apart`],
    /// which would leave it walking in and being pushed out for ever.
    ///
    /// [`World::push_apart`]: crate::game::World::push_apart
    fn bodies_touch(&self, one: Entity, other: Entity) -> bool {
        let (Some(here), Some(there)) = (
            self.transform.get(one).map(|at| at.pos),
            self.transform.get(other).map(|at| at.pos),
        ) else {
            return false;
        };
        let hulls = self.hull.get(one).map_or(Fixed::ZERO, |hull| hull.radius)
            + self.hull.get(other).map_or(Fixed::ZERO, |hull| hull.radius);
        here.within(there, hulls + rules::units(rules::STEER_MARGIN))
    }

    /// Comes round towards a spot without leaving the one it stands on.
    fn turn_to(&mut self, entity: Entity, dest: Vec2) {
        let (Some(from), Some(rate)) = (
            self.transform.get(entity).map(|t| t.pos),
            self.stats.get(entity).map(|stats| stats.turn_rate),
        ) else {
            return;
        };
        if from == dest {
            return;
        }
        let wanted = facing_towards(from, dest);
        if let Some(transform) = self.transform.get_mut(entity) {
            transform.facing = turn_towards(transform.facing, wanted, rate);
        }
    }

    /// The corner to walk at next: the destination itself when it is in plain
    /// sight, otherwise the next corner of a route laid round the buildings.
    ///
    /// Only what a player drives keeps a route; a creep walks the lane it was
    /// given and never plans around anything.
    fn next_corner(&mut self, entity: Entity, from: Vec2, dest: Vec2) -> Vec2 {
        if self.march.get(entity).is_some() {
            return dest;
        }
        // What flies is over all of it: closed ground is nothing to it, and a
        // way round it is a way round nothing.
        if self.stats.get(entity).is_some_and(|stats| stats.flies) {
            self.route.remove(entity);
            return dest;
        }
        let mut route = self.route.remove(entity).unwrap_or(Route {
            path: Vec::new(),
            goal: dest,
        });
        // A path is worth walking only to the spot it was found for. What is
        // kept here is that spot and not the last one asked for: chasing
        // something that moves a little every tick would otherwise never
        // drift far enough in one tick to be noticed, and the whole stale
        // path would be walked to where the quarry used to be.
        if !route.goal.within(dest, rules::units(rules::REPATH_DRIFT)) {
            route.path.clear();
        }
        while route
            .path
            .first()
            .is_some_and(|corner| from.within(*corner, rules::units(rules::WAYPOINT_RADIUS)))
        {
            route.path.remove(0);
        }
        if route.path.is_empty() && !grid_los(&self.grid, from, dest) {
            route.path = find_path(&self.grid, from, dest);
            route.goal = dest;
        }
        let next = route.path.first().copied().unwrap_or(dest);
        self.route.insert(entity, route);
        next
    }

    /// Whether an entity is marching a lane rather than being driven.
    pub fn is_marching(&self, entity: Entity) -> bool {
        self.march.get(entity).is_some()
    }
}

/// Where an order sends an entity, if it sends it anywhere.
fn destination(order: &UnitOrder) -> Option<Vec2> {
    match order {
        UnitOrder::Move { pos } | UnitOrder::AttackMove { pos } => Some(*pos),
        UnitOrder::Idle
        | UnitOrder::Stand
        | UnitOrder::Hold
        | UnitOrder::Attack { .. }
        | UnitOrder::Follow { .. } => None,
    }
}

impl World {
    /// Walks one entity at a spot until it stands within a reach of it.
    ///
    /// Standing near enough already, it only comes round to face the spot.
    fn walk_at(&mut self, entity: Entity, aim: Vec2, reach: bota_proto::Fixed) {
        let (Some(from), Some(stats)) = (
            self.transform.get(entity).map(|t| t.pos),
            self.stats.get(entity).copied(),
        ) else {
            return;
        };
        if from.within(aim, reach) {
            self.turn_to(entity, aim);
            return;
        }
        let waypoint = self.next_corner(entity, from, aim);
        let wanted = facing_towards(from, waypoint);
        let facing = turn_towards(
            self.transform.get(entity).expect("looked up above").facing,
            wanted,
            stats.turn_rate,
        );
        let mut next = from;
        if facing_gap(facing, wanted) <= rules::TURN_TOLERANCE_BRADS {
            next = self.walk_step(entity, waypoint, per_tick(stats.move_speed));
        }
        if let Some(transform) = self.transform.get_mut(entity) {
            transform.facing = facing;
            transform.pos = next;
        }
    }
}
