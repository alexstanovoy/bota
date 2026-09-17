//! Walking: where an entity is going, the route and the plan that take it
//! there, and the step it takes this tick.

use bota_proto::{Fixed, Vec2};

use crate::game::{
    ActionPhase, ActionState, Entity, Foreseen, LocalAsk, LocalScratch, Obstacles, Plan, Planner,
    Route, UnitOrder, World, plan_local, plan_radius,
};
use crate::game::{
    facing_gap, facing_towards, move_towards, per_tick, point_along, rules, turn_towards,
};

impl World {
    /// Turns and steps everything that has somewhere to be.
    ///
    /// Being held roots outright, and so does a channel; a swing roots
    /// whoever is making it: from the moment it begins until the recovery
    /// after it runs out, the entity does not leave the spot it stands on,
    /// though it keeps coming round to what it is swinging at.
    ///
    /// Past that, what an entity is set on decides first: with its target in
    /// reach it stands still and comes round to it, out of reach it walks at
    /// it. Only with nothing to fight does it walk where it was told.
    pub fn walk_bodies(&mut self) {
        self.lay_bodies();
        let mut scratch = std::mem::take(&mut self.local_scratch);
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            // Every mover begins the tick standing; a step taken below
            // says otherwise.
            if let Some(motion) = self.motion.get_mut(entity) {
                motion.delta = Vec2::ZERO;
                motion.still = motion.still.saturating_add(1);
            }
            // Feared, it runs from whoever put the fear on and does nothing
            // else; with nobody left to run from it stands where it is.
            if self.feared(entity) {
                match self.flees_from(entity) {
                    Some(from) => self.flee(entity, from, &mut scratch),
                    None => self.stand(entity),
                }
                continue;
            }
            // Held or channelling roots outright: there is nothing to come
            // round to.
            if self.held(entity) || self.is_channelling(entity) {
                self.stand(entity);
                continue;
            }
            // Mid-swing it comes round to what the swing was begun against;
            // recovering from one, to whatever it is set on now.
            let (rooted, face) = match self.action.get(entity).map(|action| action.state) {
                Some(ActionState::Attack {
                    target,
                    phase: ActionPhase::Before { .. },
                }) => (true, Some(target)),
                Some(ActionState::Attack { .. }) => (true, self.target_of(entity)),
                Some(
                    ActionState::CastAbility { target, .. } | ActionState::UseItem { target, .. },
                ) => (
                    true,
                    match target {
                        bota_proto::Target::Unit(target) => self.of_wire(target),
                        _ => None,
                    },
                ),
                Some(ActionState::Ready) | None => (false, None),
            };
            if rooted {
                if let Some(at) = face.and_then(|on| self.transform.get(on)).map(|t| t.pos) {
                    self.turn_to(entity, at);
                }
                self.stand(entity);
                continue;
            }
            // A cast aimed further off than it reaches walks the caster in,
            // and answers before anything else it was told to do.
            if let Some(pending) = self.pending_cast(entity)
                && let Some(aim) = self.cast_spot(pending)
            {
                let reach = self.cast_reach(entity, pending);
                if reach > 0 {
                    self.walk_toward(entity, aim, rules::units(reach), &mut scratch);
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
                    .map(|at| (at.pos, self.attack_reach(entity, on))),
                // Something it may not: reach means nothing, so it closes
                // until the bodies touch and stands there, following it for
                // as long as the order stands.
                (None, Some(on)) => self
                    .transform
                    .get(on)
                    .map(|at| (at.pos, self.touching_distance(entity, on))),
                (None, None) => None,
            };
            let holding = matches!(
                self.orders.get(entity).map(|o| o.current),
                Some(UnitOrder::Hold)
            );
            let (dest, arrive) = match on_target {
                // Holding, it comes round to what it is set on but never
                // leaves the spot it was left on.
                Some((at, _)) if holding => {
                    self.turn_to(entity, at);
                    self.stand(entity);
                    continue;
                }
                Some((at, reach)) => (at, reach),
                None => {
                    let Some(dest) = self
                        .orders
                        .get(entity)
                        .and_then(|o| destination(&o.current))
                    else {
                        self.stand(entity);
                        continue;
                    };
                    (dest, Fixed::ZERO)
                }
            };
            self.walk_toward(entity, dest, arrive, &mut scratch);
        }
        self.recycle_entity_snapshot(entities);
        self.local_scratch = scratch;
    }

    /// How near an attacker's centre has to come to a target's to strike
    /// it: its attack range and both bound radii.
    fn attack_reach(&self, attacker: Entity, target: Entity) -> Fixed {
        let range = self
            .stats
            .get(attacker)
            .map_or(Fixed::ZERO, |stats| stats.attack_range);
        range
            + self.hull.get(attacker).map_or(Fixed::ZERO, |h| h.bound)
            + self.hull.get(target).map_or(Fixed::ZERO, |h| h.bound)
    }

    /// How near two bodies' centres are when the bodies touch: a hair
    /// further apart than the hulls themselves, so what stops here is not
    /// overlapping and is not eased away again by [`World::push_apart`].
    fn touching_distance(&self, one: Entity, other: Entity) -> Fixed {
        self.hull
            .get(one)
            .map_or(Fixed::ZERO, |hull| hull.collision)
            + self
                .hull
                .get(other)
                .map_or(Fixed::ZERO, |hull| hull.collision)
            + rules::units(rules::STEER_MARGIN)
    }

    /// Stands this tick: whatever stretch of walk it had planned is
    /// forgotten, so nobody plans round where it was going to be.
    fn stand(&mut self, entity: Entity) {
        if let Some(plan) = self.plan.get_mut(entity) {
            plan.clear();
        }
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

    /// Whether an entity is marching a lane rather than being driven.
    pub fn is_marching(&self, entity: Entity) -> bool {
        self.march.get(entity).is_some()
    }

    /// Forgets the way an entity was walking: what put it somewhere else
    /// calls this, and the walk is laid again from where it now stands.
    pub fn forget_walk(&mut self, entity: Entity) {
        if let Some(route) = self.route.get_mut(entity) {
            *route = Route::none();
        }
        if let Some(plan) = self.plan.get_mut(entity) {
            plan.clear();
        }
    }

    /// Walks one entity towards a spot until it stands within a reach of
    /// it, one tick's worth.
    ///
    /// Standing near enough already, it only comes round to face the spot.
    /// What flies goes straight; what walks follows its route round what
    /// stands still and its plan round what moves, and takes the plan's
    /// next step unless a body has come to stand in it.
    fn walk_toward(
        &mut self,
        entity: Entity,
        dest: Vec2,
        arrive: Fixed,
        scratch: &mut LocalScratch,
    ) {
        let (Some(from), Some(stats)) = (
            self.transform.get(entity).map(|t| t.pos),
            self.stats.get(entity).copied(),
        ) else {
            return;
        };
        if from == dest || (arrive > Fixed::ZERO && from.within(dest, arrive)) {
            self.turn_to(entity, dest);
            self.stand(entity);
            return;
        }
        let step = per_tick(stats.move_speed);
        if step <= Fixed::ZERO {
            self.stand(entity);
            return;
        }
        if stats.flies {
            self.fly_toward(entity, dest, step);
            return;
        }
        let now = self.tick;
        let collision = self.hull.get(entity).map_or(Fixed::ZERO, |h| h.collision);
        let motion = self.motion.get(entity).copied().unwrap_or_default();
        // Just walked into a body that was itself moving: it stands for
        // the block wait, facing where it was going.
        if motion.wait_until > now {
            self.turn_to(entity, dest);
            if let Some(motion) = self.motion.get_mut(entity) {
                motion.stalled = motion.stalled.saturating_add(1);
            }
            return;
        }
        let (mut aim, mut last) = self.lay_route(entity, from, dest, collision, &[]);
        if self.route.get(entity).is_some_and(|route| route.done) {
            self.turn_to(entity, dest);
            self.stand(entity);
            return;
        }
        // A walk that ends within a reach of its destination is judged
        // against the destination itself, not the spot beside it the route
        // ends on: a tower's centre cannot be stood on, but its reach is
        // measured from there.
        if last && arrive > Fixed::ZERO {
            aim = dest;
        }
        let aim_arrive = if last {
            arrive
        } else {
            rules::units(rules::WAYPOINT_RADIUS)
        };
        let marching = self.march.get(entity).is_some();
        let shoving = marching && motion.stalled >= rules::MARCH_SHOVE_TICKS;
        // The plan round what moves: kept while it still leads where the
        // route goes and has enough left, laid again otherwise, though not
        // too often.
        let goal = self
            .route
            .get(entity)
            .and_then(|route| route.goal)
            .unwrap_or(dest);
        // A plan is walked from where it was laid: a body put somewhere
        // else since, by a hook or a shove, has no plan. The check reads the
        // plan in place; cloning it only to test it bought nothing.
        if plan_stale(self.plan.get(entity), from, step, goal, motion.stalled, now) {
            let (mut steps, crowded, mut reached) =
                self.lay_plan(entity, from, aim, aim_arrive, collision, step, scratch);
            // Short of the aim with bodies about, the route is laid round
            // the bodies standing there as if they were structures, and
            // the plan laid again along it: a wall of them is too wide for
            // the plan to find its way round on its own.
            if !reached && crowded && now >= motion.relaid + rules::STALL_RELAY_GAP {
                let mut extra = std::mem::take(&mut self.extra_scratch);
                extra.clear();
                self.standing_about(entity, from, &mut extra);
                if !extra.is_empty() {
                    if let Some(motion) = self.motion.get_mut(entity) {
                        motion.relaid = now;
                    }
                    let (mut aim2, last2) = self.lay_route(entity, from, dest, collision, &extra);
                    if last2 && arrive > Fixed::ZERO {
                        aim2 = dest;
                    }
                    let arrive2 = if last2 {
                        arrive
                    } else {
                        rules::units(rules::WAYPOINT_RADIUS)
                    };
                    let (steps2, _, reached2) =
                        self.lay_plan(entity, from, aim2, arrive2, collision, step, scratch);
                    if reached2 || steps2.len() > steps.len() {
                        steps = steps2;
                        aim = aim2;
                        last = last2;
                        reached = reached2;
                    }
                }
                self.extra_scratch = extra;
            }
            let empty = steps.is_empty();
            self.plan.insert(
                entity,
                Plan {
                    steps,
                    from: now + 1,
                    at: 0,
                    goal,
                    step,
                    laid: now,
                    last,
                },
            );
            if empty && reached && !last {
                // Standing on a spot of the route already: the next tick
                // aims past it.
                if let Some(route) = self.route.get_mut(entity)
                    && route.corners.len() > 1
                {
                    route.corners.remove(0);
                }
                self.turn_to(entity, dest);
                return;
            }
            if empty {
                // Nothing gets it nearer. At the end of its route with
                // nobody about, that is the ground's last word and the walk
                // is over; with bodies about it is stuck for now, and asks
                // again once they have moved.
                let over = last && !crowded;
                if over {
                    if let Some(route) = self.route.get_mut(entity) {
                        route.done = true;
                    }
                    self.turn_to(entity, dest);
                    return;
                }
                if shoving {
                    self.shove(entity, from, aim, step, collision);
                    return;
                }
                self.stall(entity, dest);
                return;
            }
        }
        let Some(target) = self
            .plan
            .get(entity)
            .and_then(|plan| plan.steps.get(plan.at).copied())
        else {
            if shoving {
                self.shove(entity, from, aim, step, collision);
                return;
            }
            self.stall(entity, dest);
            return;
        };
        if target == from {
            // A tick stood turning onto the next stretch.
            let upcoming = self.plan.get(entity).and_then(|plan| {
                plan.steps[plan.at..]
                    .iter()
                    .find(|spot| **spot != from)
                    .copied()
            });
            self.turn_to(entity, upcoming.unwrap_or(dest));
            if let Some(plan) = self.plan.get_mut(entity) {
                plan.at += 1;
            }
            return;
        }
        if !shoving
            && !self.phased(entity)
            && let Some(blocker) = self.body_in_the_way(entity, from, target)
        {
            // Whoever stands in the step was not where the plan had them.
            // A body that is itself moving costs the block wait; a standing
            // one is planned round again straight away.
            let moving = self
                .motion
                .get(blocker)
                .is_some_and(|theirs| theirs.delta != Vec2::ZERO)
                || self.plan.get(blocker).is_some_and(|theirs| theirs.stands());
            let hero = self.hero.get(blocker).is_some();
            if let Some(motion) = self.motion.get_mut(entity) {
                if moving {
                    motion.wait_until = now + rules::BLOCK_WAIT_TICKS;
                }
                if hero {
                    motion.bumps = if now < motion.bumped + rules::HEED_HERO_TICKS {
                        motion.bumps.saturating_add(1)
                    } else {
                        1
                    };
                    motion.bumped = now;
                }
            }
            if let Some(plan) = self.plan.get_mut(entity) {
                plan.clear();
            }
            self.turn_to(entity, target);
            self.stall(entity, dest);
            return;
        }
        self.take_step(entity, from, target);
        if let Some(plan) = self.plan.get_mut(entity) {
            plan.at += 1;
        }
    }

    /// Lays the next stretch of a walk: the plan from where the entity
    /// stands to its aim, round the bodies about it and where they are
    /// going. Answers whether any body was about to be planned round, and
    /// whether the plan gets to the aim.
    #[expect(
        clippy::too_many_arguments,
        reason = "a plan is asked for by this many things"
    )]
    fn lay_plan(
        &self,
        entity: Entity,
        from: Vec2,
        aim: Vec2,
        arrive: Fixed,
        collision: Fixed,
        step: Fixed,
        scratch: &mut LocalScratch,
    ) -> (Vec<Vec2>, bool, bool) {
        let ob = Obstacles {
            field: &self.clearance,
            extra: &[],
        };
        let now = self.tick;
        let facing = self
            .transform
            .get(entity)
            .map_or(bota_proto::Angle::default(), |t| t.facing);
        let turn_rate = self.stats.get(entity).map_or(0, |stats| stats.turn_rate);
        let ask = LocalAsk {
            from,
            facing,
            goal: aim,
            arrive,
            radius: collision,
            step,
            turn_rate,
            now: self.tick,
        };
        let mut foreseen = Vec::new();
        if !self.phased(entity) {
            let reach = Fixed {
                raw: step.raw.saturating_mul(rules::LOCAL_HORIZON_TICKS as i32),
            } + rules::units(rules::LOCAL_BODIES_PAD);
            self.bodies.near(from, reach, |body| {
                if body.entity == entity || self.phased(body.entity) {
                    return;
                }
                let Some(at) = self.transform.get(body.entity).map(|t| t.pos) else {
                    return;
                };
                // A hero is foreseen where it stands, and only when it has
                // stood there a while or has been run into again and again
                // of late: it goes where a player sends it next, which
                // nothing here knows, and one on the move is met when it is
                // met, and tried again straight after.
                if self.hero.get(body.entity).is_some() {
                    let steady = body.still >= rules::STANDING_TICKS;
                    let bumped = self.motion.get(entity).is_some_and(|mine| {
                        now < mine.bumped + rules::HEED_HERO_TICKS
                            && mine.bumps >= rules::BUMPS_BEFORE_HEED
                    });
                    if steady || bumped {
                        foreseen.push(Foreseen {
                            at,
                            radius: body.radius,
                            delta: Vec2::ZERO,
                            steps: &[],
                            from: 0,
                        });
                    }
                    return;
                }
                let (steps, plan_from) = self
                    .plan
                    .get(body.entity)
                    .filter(|plan| plan.stands())
                    .map_or((&[][..], 0), |plan| (plan.steps.as_slice(), plan.from));
                foreseen.push(Foreseen {
                    at,
                    radius: body.radius,
                    delta: body.delta,
                    steps,
                    from: plan_from,
                });
            });
        }
        let crowded = !foreseen.is_empty();
        let (steps, reached) = plan_local(&ob, &foreseen, &ask, scratch);
        (steps, crowded, reached)
    }

    /// Every body standing still about a spot that is not a structure,
    /// as circles a route may be laid round.
    fn standing_about(&self, entity: Entity, from: Vec2, out: &mut Vec<(Vec2, Fixed)>) {
        let reach = rules::units(rules::STANDING_REACH);
        self.bodies.near(from, reach, |body| {
            if body.entity == entity || body.fixed || body.still < rules::STANDING_TICKS {
                return;
            }
            if let Some(at) = self.transform.get(body.entity).map(|t| t.pos) {
                out.push((at, body.radius));
            }
        });
    }

    /// Keeps an entity's route to a destination: laid afresh when the
    /// destination is new or has drifted, its last leg swung onto a
    /// destination that moved a little, its corners dropped as they are
    /// passed, and laid again from where the entity stands when the way
    /// to its next corner is shut.
    ///
    /// Answers the spot the entity walks at next: as far along the route as
    /// [`rules::LOCAL_REACH`] and a straight line from where it stands
    /// allow, and whether that spot is the route's end.
    fn lay_route(
        &mut self,
        entity: Entity,
        from: Vec2,
        dest: Vec2,
        collision: Fixed,
        extra: &[(Vec2, Fixed)],
    ) -> (Vec2, bool) {
        let room = plan_radius(collision);
        let mut route = self
            .route
            .get_mut(entity)
            .map_or_else(Route::none, |route| std::mem::replace(route, Route::none()));
        let ob = Obstacles {
            field: &self.clearance,
            extra,
        };
        let mut fresh = !extra.is_empty();
        match route.goal {
            None => fresh = true,
            Some(goal) if goal == dest => {}
            Some(goal) if !goal.within(dest, rules::units(rules::REPATH_DRIFT)) => fresh = true,
            Some(_) => {
                // A little drift: the last leg swings onto the new goal
                // when it can, else the route is laid afresh.
                let anchor = if route.corners.len() >= 2 {
                    route.corners[route.corners.len() - 2]
                } else {
                    from
                };
                if ob.clear(anchor, dest, room) {
                    if let Some(last) = route.corners.last_mut() {
                        *last = dest;
                    }
                    route.end = dest;
                    route.goal = Some(dest);
                    route.done = false;
                } else {
                    fresh = true;
                }
            }
        }
        if fresh {
            relay(&mut self.planner, &mut route, &ob, from, dest, collision);
        }
        // What the body itself can walk is judged at its own size: the
        // route keeps a margin off everything, and a body pressed against
        // a tower sees nothing along it at that margin.
        pass_corners(&ob, &mut route, from, collision);
        // Pushed off the route with no straight way back to its next
        // corner, it is laid again from here.
        if !fresh
            && let Some(first) = route.corners.first().copied()
            && !ob.clear(from, first, collision)
        {
            relay(&mut self.planner, &mut route, &ob, from, dest, collision);
            pass_corners(&ob, &mut route, from, collision);
        }
        // Beside the end of a route that stops short of its goal, the last
        // stretch aims at the goal itself: the route keeps a margin the
        // body does not, and the walk gets as near as the body lets it.
        if route.corners.len() == 1
            && route.end != dest
            && from.within(route.end, rules::units(rules::WAYPOINT_RADIUS))
        {
            route.corners[0] = dest;
            route.end = dest;
        }
        let end = route.end;
        let points: &[Vec2] = if route.corners.is_empty() {
            std::slice::from_ref(&end)
        } else {
            &route.corners
        };
        let aim = visible_along(
            &ob,
            from,
            points,
            rules::units(rules::LOCAL_REACH),
            collision,
        );
        if let Some(slot) = self.route.get_mut(entity) {
            *slot = route;
        } else {
            self.route.insert(entity, route);
        }
        aim
    }

    /// Takes a step: the body moves and comes round towards the way it
    /// went.
    fn take_step(&mut self, entity: Entity, from: Vec2, to: Vec2) {
        let rate = self.stats.get(entity).map_or(0, |stats| stats.turn_rate);
        if let Some(transform) = self.transform.get_mut(entity) {
            transform.facing = turn_towards(transform.facing, facing_towards(from, to), rate);
            transform.pos = to;
        }
        if let Some(motion) = self.motion.get_mut(entity) {
            motion.delta = to - from;
            motion.still = 0;
            motion.stalled = 0;
        }
    }

    /// A tick stood wanting to move and unable to.
    fn stall(&mut self, entity: Entity, dest: Vec2) {
        self.turn_to(entity, dest);
        if let Some(motion) = self.motion.get_mut(entity) {
            motion.stalled = motion.stalled.saturating_add(1);
        }
    }

    /// A creep that has stood stalled long enough walks into the bodies in
    /// its way, straight at its aim, and is eased out of them after. Closed
    /// ground stops it all the same.
    fn shove(&mut self, entity: Entity, from: Vec2, aim: Vec2, step: Fixed, collision: Fixed) {
        let next = move_towards(from, aim, step);
        if next != from && self.clearance.capsule_clear(from, next, collision) {
            self.take_step(entity, from, next);
        } else {
            self.stall(entity, aim);
        }
    }

    /// What flies goes straight: closed ground and bodies are nothing to
    /// it. It turns first, like everything else.
    fn fly_toward(&mut self, entity: Entity, dest: Vec2, step: Fixed) {
        let (Some(from), Some(rate)) = (
            self.transform.get(entity).map(|t| t.pos),
            self.stats.get(entity).map(|stats| stats.turn_rate),
        ) else {
            return;
        };
        let wanted = facing_towards(from, dest);
        let facing = turn_towards(
            self.transform.get(entity).expect("looked up above").facing,
            wanted,
            rate,
        );
        if let Some(transform) = self.transform.get_mut(entity) {
            transform.facing = facing;
        }
        if facing_gap(facing, wanted) <= rules::TURN_TOLERANCE_BRADS {
            let next = crate::game::clamp_to_map(move_towards(from, dest, step));
            self.take_step(entity, from, next);
        }
    }

    /// Runs one entity straight away from another, a step a tick.
    fn flee(&mut self, entity: Entity, from: Entity, scratch: &mut LocalScratch) {
        let (Some(here), Some(there)) = (
            self.transform.get(entity).map(|t| t.pos),
            self.transform.get(from).map(|t| t.pos),
        ) else {
            return;
        };
        if here == there {
            return;
        }
        let away = crate::game::clamp_to_map(point_along(
            here,
            here + (here - there),
            Fixed::from_int(rules::FLEE_LOOKAHEAD),
        ));
        self.walk_toward(entity, away, Fixed::ZERO, scratch);
    }
}

/// Drops the corners of a route a body has passed: it stands beside one,
/// or beside or beyond it along the leg to the next with that next in a
/// straight line at the body's own size. The end is never dropped.
fn pass_corners(ob: &Obstacles, route: &mut Route, from: Vec2, collision: Fixed) {
    while route.corners.len() > 1 {
        let (here, next) = (route.corners[0], route.corners[1]);
        let beyond = {
            let ax = i64::from(from.x.raw) - i64::from(here.x.raw);
            let ay = i64::from(from.y.raw) - i64::from(here.y.raw);
            let lx = i64::from(next.x.raw) - i64::from(here.x.raw);
            let ly = i64::from(next.y.raw) - i64::from(here.y.raw);
            ax * lx + ay * ly > 0
        };
        let passed = from.within(here, rules::units(rules::WAYPOINT_RADIUS))
            || (beyond && ob.clear(from, next, collision));
        if !passed {
            break;
        }
        route.corners.remove(0);
    }
}

/// Lays a route afresh from a spot to a destination: no corners when the
/// destination is in a straight line, else the corners found.
fn relay(
    planner: &mut Planner,
    route: &mut Route,
    ob: &Obstacles,
    from: Vec2,
    dest: Vec2,
    collision: Fixed,
) {
    route.goal = Some(dest);
    route.done = false;
    if ob.clear(from, dest, plan_radius(collision)) {
        route.corners.clear();
        route.end = dest;
    } else {
        let budget = if ob.extra.is_empty() {
            rules::PATH_EXPANSIONS
        } else {
            rules::STALL_PATH_EXPANSIONS
        };
        route.corners = planner.find_path_within(ob, from, dest, collision, budget);
        route.end = route.corners.last().copied().unwrap_or(from);
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

/// Whether the laid plan still leads where the route goes and has enough left.
///
/// A plan that was never laid reads as stale: its step is zero against a
/// positive step per tick, exactly as the clone-based check read it.
fn plan_stale(
    plan: Option<&Plan>,
    from: Vec2,
    step: Fixed,
    goal: Vec2,
    stalled: u32,
    now: u32,
) -> bool {
    // Stalled long, it asks for a plan less often.
    let pause = if stalled >= rules::STALL_BACKOFF_AFTER {
        rules::REPLAN_STALLED_TICKS
    } else {
        rules::REPLAN_MIN_TICKS
    };
    let Some(plan) = plan else {
        return true;
    };
    let astray = plan
        .steps
        .get(plan.at)
        .is_some_and(|next| !next.within(from, step + rules::units(rules::PLAN_STRAY)));
    let rested = now >= plan.laid + pause;
    astray
        || plan.step != step
        || !plan.goal.within(goal, rules::units(rules::PLAN_DRIFT))
        || (plan.steps.is_empty() && rested)
        || (!plan.stands() && !plan.steps.is_empty())
        || (plan.left() < rules::REPLAN_LEFT_TICKS as usize && !plan.last && rested)
}

/// The spot to walk at next along a polyline from a position: as far along
/// it as a reach allows, and no further than a straight line from the
/// position stays clear, with whether that spot is the polyline's end.
///
/// The first point itself when not even it is in a straight line: the walk
/// has to work its way there.
fn visible_along(
    ob: &Obstacles,
    from: Vec2,
    points: &[Vec2],
    reach: Fixed,
    room: Fixed,
) -> (Vec2, bool) {
    let mut at = from;
    let mut left = i64::from(reach.raw);
    let mut best: Option<(Vec2, bool)> = None;
    for (i, &point) in points.iter().enumerate() {
        let leg = crate::game::isqrt64(at.distance_squared(point));
        if leg > left {
            let spot = point_along(at, point, Fixed { raw: left as i32 });
            if ob.clear(from, spot, room) {
                return (spot, false);
            }
            break;
        }
        if !ob.clear(from, point, room) {
            break;
        }
        best = Some((point, i + 1 == points.len()));
        left -= leg;
        at = point;
    }
    best.unwrap_or((points[0], points.len() == 1))
}
