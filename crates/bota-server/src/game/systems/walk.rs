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
use crate::profile::Phase;

impl World {
    /// Turns and steps everything that has somewhere to be.
    pub fn walk_bodies(&mut self) {
        let _profile = self.scope(Phase::Walk);
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
            self.walk_one(entity, &mut scratch);
        }
        self.recycle_entity_snapshot(entities);
        self.local_scratch = scratch;
    }

    /// One entity's tick of walking.
    ///
    /// Feared, it runs from whoever put the fear on. Held or channelling it
    /// stands; mid-swing, or casting, it stands and comes round to what it
    /// acts on. A cast aimed further off than it reaches walks it in. Past
    /// that, what it is set on decides: in reach it stands and comes round
    /// to it, out of reach it walks at it. Only with nothing to fight does
    /// it walk where it was told.
    fn walk_one(&mut self, entity: Entity, scratch: &mut LocalScratch) {
        if self.feared(entity) {
            match self.flees_from(entity) {
                Some(from) => self.flee(entity, from, scratch),
                None => self.stand(entity),
            }
            return;
        }
        if self.held(entity) || self.is_channelling(entity) {
            self.stand(entity);
            return;
        }
        if let Some(face) = self.acting_on(entity) {
            if let Some(at) = face.and_then(|on| self.transform.get(on)).map(|t| t.pos) {
                self.turn_to(entity, at);
            }
            self.stand(entity);
            return;
        }
        if let Some(pending) = self.pending_cast(entity)
            && let Some(aim) = self.cast_spot(pending)
        {
            let reach = self.cast_reach(entity, pending);
            if reach > 0 {
                self.walk_toward(entity, aim, rules::units(reach), scratch);
                return;
            }
        }
        let holding = matches!(
            self.orders.get(entity).map(|o| o.current),
            Some(UnitOrder::Hold)
        );
        match self.closing_on(entity) {
            // Holding, it comes round to what it is set on but never leaves
            // the spot it was left on.
            Some((at, _)) if holding => {
                self.turn_to(entity, at);
                self.stand(entity);
            }
            Some((at, reach)) => self.walk_toward(entity, at, reach, scratch),
            None => match self
                .orders
                .get(entity)
                .and_then(|o| destination(&o.current))
            {
                Some(dest) => self.walk_toward(entity, dest, Fixed::ZERO, scratch),
                None => self.stand(entity),
            },
        }
    }

    /// What an entity mid-action comes round to, when an action roots it:
    /// what a swing was begun against, while it is being begun; what it is
    /// set on now, while it recovers; what a cast is aimed at, if a unit.
    fn acting_on(&self, entity: Entity) -> Option<Option<Entity>> {
        match self.action.get(entity).map(|action| action.state) {
            Some(ActionState::Attack {
                target,
                phase: ActionPhase::Before { .. },
            }) => Some(Some(target)),
            Some(ActionState::Attack { .. }) => Some(self.target_of(entity)),
            Some(ActionState::CastAbility { target, .. } | ActionState::UseItem { target, .. }) => {
                Some(match target {
                    bota_proto::Target::Unit(target) => self.of_wire(target),
                    _ => None,
                })
            }
            Some(ActionState::Ready) | None => None,
        }
    }

    /// Where the one an entity is set on stands, and how near it comes to
    /// it: to its reach for something it may strike, until the bodies touch
    /// for something it follows on an order.
    fn closing_on(&self, entity: Entity) -> Option<(Vec2, Fixed)> {
        let ordered_at = match self.orders.get(entity).map(|o| o.current) {
            Some(UnitOrder::Attack { target, .. } | UnitOrder::Follow { target, .. }) => {
                Some(target)
            }
            _ => None,
        };
        let chosen = self.target_of(entity).filter(|on| self.alive(*on));
        let ordered = ordered_at.filter(|on| self.alive(*on));
        match (chosen, ordered) {
            (Some(on), _) => self
                .transform
                .get(on)
                .map(|at| (at.pos, self.attack_reach(entity, on))),
            (None, Some(on)) => self
                .transform
                .get(on)
                .map(|at| (at.pos, self.touching_distance(entity, on))),
            (None, None) => None,
        }
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
        let Some((from, step)) = self.walking_pace(entity, dest, arrive) else {
            return;
        };
        let collision = self.hull.get(entity).map_or(Fixed::ZERO, |h| h.collision);
        let motion = self.motion.get(entity).copied().unwrap_or_default();
        // Just walked into a body that was itself moving: it stands for
        // the block wait, facing where it was going.
        if motion.wait_until > self.tick {
            self.turn_to(entity, dest);
            if let Some(motion) = self.motion.get_mut(entity) {
                motion.stalled = motion.stalled.saturating_add(1);
            }
            return;
        }
        let (aim, last) = self.lay_route(entity, from, dest, collision, &[]);
        if self.route.get(entity).is_some_and(|route| route.done) {
            self.turn_to(entity, dest);
            self.stand(entity);
            return;
        }
        let mut leg = Leg {
            entity,
            from,
            dest,
            arrive,
            step,
            collision,
            // A walk that ends within a reach of its destination is judged
            // against the destination itself, not the spot beside it the
            // route ends on: a tower's centre cannot be stood on, but its
            // reach is measured from there.
            aim: if last && arrive > Fixed::ZERO {
                dest
            } else {
                aim
            },
            last,
            shoving: self.march.get(entity).is_some() && motion.stalled >= rules::MARCH_SHOVE_TICKS,
            before_relay: None,
        };
        let goal = self
            .route
            .get(entity)
            .and_then(|route| route.goal)
            .unwrap_or(dest);
        // A plan is walked from where it was laid: a body put somewhere
        // else since, by a hook or a shove, has no plan.
        if plan_stale(
            self.plan.get(entity),
            from,
            step,
            goal,
            motion.stalled,
            self.tick,
        ) && self.relay_plan(&mut leg, goal, motion, scratch)
        {
            return;
        }
        self.follow_plan(&leg);
    }

    /// Where a walker stands and how far it walks this tick, or nothing
    /// when it does not walk: near enough already, it comes round to face
    /// the spot; unable to move, it stands; flying, it goes straight.
    fn walking_pace(&mut self, entity: Entity, dest: Vec2, arrive: Fixed) -> Option<(Vec2, Fixed)> {
        let from = self.transform.get(entity)?.pos;
        let stats = self.stats.get(entity).copied()?;
        if from == dest || (arrive > Fixed::ZERO && from.within(dest, arrive)) {
            self.turn_to(entity, dest);
            self.stand(entity);
            return None;
        }
        let step = per_tick(stats.move_speed);
        if step <= Fixed::ZERO {
            self.stand(entity);
            return None;
        }
        if stats.flies {
            self.fly_toward(entity, dest, step);
            return None;
        }
        Some((from, step))
    }

    /// Lays a walker's plan afresh along its route, round the bodies about
    /// it, and answers whether that settles its walk this tick.
    fn relay_plan(
        &mut self,
        leg: &mut Leg,
        goal: Vec2,
        motion: crate::game::Motion,
        scratch: &mut LocalScratch,
    ) -> bool {
        let (entity, now) = (leg.entity, self.tick);
        let (mut steps, crowded, mut reached) = self.lay_plan(leg, scratch);
        // Short of the aim with bodies about, the route is laid round the
        // bodies standing there as if they were structures.
        if !reached && crowded && now >= motion.relaid + rules::STALL_RELAY_GAP {
            match self.relay_round_standing(leg, scratch) {
                (Some(round), reached_round) if reached_round || round.len() > steps.len() => {
                    steps = round;
                    reached = reached_round;
                }
                (Some(_), _) => leg.undo_relay(),
                (None, _) => {}
            }
        }
        let empty = steps.is_empty();
        self.plan.insert(
            entity,
            Plan {
                steps,
                from: now + 1,
                at: 0,
                goal,
                step: leg.step,
                laid: now,
                last: leg.last,
            },
        );
        if !empty {
            return false;
        }
        self.walk_nowhere(leg, reached, crowded);
        true
    }

    /// A walker whose fresh plan takes it nowhere: standing on a spot of
    /// its route already, at the end of its walk, or stuck for now.
    fn walk_nowhere(&mut self, leg: &Leg, reached: bool, crowded: bool) {
        let entity = leg.entity;
        if reached && !leg.last {
            // Standing on a spot of the route already: the next tick aims
            // past it.
            if let Some(route) = self.route.get_mut(entity)
                && route.corners.len() > 1
            {
                route.corners.remove(0);
            }
            self.turn_to(entity, leg.dest);
            return;
        }
        // Nothing gets it nearer. At the end of its route with nobody
        // about, that is the ground's last word and the walk is over; with
        // bodies about it is stuck for now, and asks again once they have
        // moved.
        if leg.last && !crowded {
            if let Some(route) = self.route.get_mut(entity) {
                route.done = true;
            }
            self.turn_to(entity, leg.dest);
            return;
        }
        self.shove_or_stall(leg);
    }

    /// Lays a walker's route round the bodies standing still about it, and
    /// its plan along that route, trying the route's new aim on the leg.
    /// Answers nothing when nobody stands about, else the plan and whether
    /// it gets to the aim.
    fn relay_round_standing(
        &mut self,
        leg: &mut Leg,
        scratch: &mut LocalScratch,
    ) -> (Option<Vec<Vec2>>, bool) {
        let mut extra = std::mem::take(&mut self.extra_scratch);
        extra.clear();
        self.standing_about(leg.entity, leg.from, &mut extra);
        if extra.is_empty() {
            self.extra_scratch = extra;
            return (None, false);
        }
        if let Some(motion) = self.motion.get_mut(leg.entity) {
            motion.relaid = self.tick;
        }
        let (aim, last) = self.lay_route(leg.entity, leg.from, leg.dest, leg.collision, &extra);
        self.extra_scratch = extra;
        leg.try_relay(aim, last);
        let (steps, _, reached) = self.lay_plan(leg, scratch);
        (Some(steps), reached)
    }

    /// Takes the next step of a walker's plan, unless it turns onto the
    /// next stretch, or a body stands in the step.
    fn follow_plan(&mut self, leg: &Leg) {
        let (entity, from) = (leg.entity, leg.from);
        let Some(target) = self
            .plan
            .get(entity)
            .and_then(|plan| plan.steps.get(plan.at).copied())
        else {
            self.shove_or_stall(leg);
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
            self.turn_to(entity, upcoming.unwrap_or(leg.dest));
            if let Some(plan) = self.plan.get_mut(entity) {
                plan.at += 1;
            }
            return;
        }
        if !leg.shoving
            && !self.phased(entity)
            && let Some(blocker) = self.body_in_the_way(entity, from, target)
        {
            self.blocked_by(entity, blocker);
            self.turn_to(entity, target);
            self.stall(entity, leg.dest);
            return;
        }
        self.take_step(entity, from, target);
        if let Some(plan) = self.plan.get_mut(entity) {
            plan.at += 1;
        }
    }

    /// Whoever stands in a walker's step was not where its plan had them:
    /// a body that is itself moving costs the block wait, a standing one is
    /// planned round again straight away, and a hero run into is counted.
    fn blocked_by(&mut self, entity: Entity, blocker: Entity) {
        let now = self.tick;
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
    }

    /// A walker that goes nowhere this tick: shoving into what is in its way
    /// when it has stood stalled long enough, else stalled.
    fn shove_or_stall(&mut self, leg: &Leg) {
        if leg.shoving {
            self.shove(leg.entity, leg.from, leg.aim, leg.step, leg.collision);
        } else {
            self.stall(leg.entity, leg.dest);
        }
    }

    /// Lays the next stretch of a walk: the plan from where the walker
    /// stands to its aim, round the bodies about it and where they are
    /// going. Answers whether any body was about to be planned round, and
    /// whether the plan gets to the aim.
    fn lay_plan(&self, leg: &Leg, scratch: &mut LocalScratch) -> (Vec<Vec2>, bool, bool) {
        let _profile = self.scope(Phase::LocalPlan);
        let ob = Obstacles {
            field: &self.clearance,
            extra: &[],
        };
        let ask = LocalAsk {
            from: leg.from,
            facing: self
                .transform
                .get(leg.entity)
                .map_or(bota_proto::Angle::default(), |t| t.facing),
            goal: leg.aim,
            arrive: leg.aim_arrive(),
            radius: leg.collision,
            step: leg.step,
            turn_rate: self
                .stats
                .get(leg.entity)
                .map_or(0, |stats| stats.turn_rate),
            now: self.tick,
        };
        let foreseen = self.foreseen_about(leg.entity, leg.from, leg.step);
        let crowded = !foreseen.is_empty();
        let _search_profile = self.scope(Phase::LocalSearch);
        let (steps, reached) = plan_local(&ob, &foreseen, &ask, scratch);
        (steps, crowded, reached)
    }

    /// Every body a walker's plan is laid round, and where each will be.
    ///
    /// A hero is foreseen where it stands, and only when it has stood there
    /// a while or has been run into again and again of late. Nothing is
    /// foreseen for a phased walker, and a phased body is walked through.
    fn foreseen_about(&self, entity: Entity, from: Vec2, step: Fixed) -> Vec<Foreseen<'_>> {
        let mut foreseen = Vec::new();
        if self.phased(entity) {
            return foreseen;
        }
        let now = self.tick;
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
        foreseen
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
        let _profile = self.scope(Phase::Route);
        let room = plan_radius(collision);
        let mut route = self
            .route
            .get_mut(entity)
            .map_or_else(Route::none, |route| std::mem::replace(route, Route::none()));
        let ob = Obstacles {
            field: &self.clearance,
            extra,
        };
        let kept = keeps_route(&ob, &mut route, from, dest, room);
        let fresh = !kept || !extra.is_empty();
        if fresh {
            let _path_profile = self.scope(Phase::PathQuery);
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
            let _path_profile = self.scope(Phase::PathQuery);
            relay(&mut self.planner, &mut route, &ob, from, dest, collision);
            drop(_path_profile);
            pass_corners(&ob, &mut route, from, collision);
        }
        let aim = next_aim(&ob, &mut route, from, dest, collision);
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

/// One walker's tick of walking along its route.
struct Leg {
    /// Who walks.
    entity: Entity,
    /// Where it stands.
    from: Vec2,
    /// Where it is going.
    dest: Vec2,
    /// How near the destination is near enough.
    arrive: Fixed,
    /// How far it walks in a tick.
    step: Fixed,
    /// Its collision size.
    collision: Fixed,
    /// The spot it walks at next.
    aim: Vec2,
    /// Whether that spot is the route's end.
    last: bool,
    /// Whether it walks into the bodies in its way.
    shoving: bool,
    /// The aim and end it had before a route laid round standing bodies
    /// was tried, while one is.
    before_relay: Option<(Vec2, bool)>,
}

impl Leg {
    /// How near the aim is near enough: the destination's own reach at the
    /// route's end, a waypoint's short of it.
    fn aim_arrive(&self) -> Fixed {
        if self.last {
            self.arrive
        } else {
            rules::units(rules::WAYPOINT_RADIUS)
        }
    }

    /// Tries the aim of a route laid afresh, keeping what it replaces.
    fn try_relay(&mut self, aim: Vec2, last: bool) {
        self.before_relay = Some((self.aim, self.last));
        self.aim = if last && self.arrive > Fixed::ZERO {
            self.dest
        } else {
            aim
        };
        self.last = last;
    }

    /// Goes back to the aim a tried route replaced.
    fn undo_relay(&mut self) {
        if let Some((aim, last)) = self.before_relay.take() {
            self.aim = aim;
            self.last = last;
        }
    }
}

/// Whether a route still serves a destination: as it is for the one it was
/// laid to, and with its last leg swung onto one that drifted a little when
/// that leg stays clear. Anything else asks for a route laid afresh.
fn keeps_route(ob: &Obstacles, route: &mut Route, from: Vec2, dest: Vec2, room: Fixed) -> bool {
    match route.goal {
        None => false,
        Some(goal) if goal == dest => true,
        Some(goal) if !goal.within(dest, rules::units(rules::REPATH_DRIFT)) => false,
        Some(_) => {
            let anchor = if route.corners.len() >= 2 {
                route.corners[route.corners.len() - 2]
            } else {
                from
            };
            if !ob.clear(anchor, dest, room) {
                return false;
            }
            if let Some(last) = route.corners.last_mut() {
                *last = dest;
            }
            route.end = dest;
            route.goal = Some(dest);
            route.done = false;
            true
        }
    }
}

/// The spot to walk at next along a route, and whether it is the route's
/// end.
///
/// Beside the end of a route that stops short of its goal, the last stretch
/// aims at the goal itself: the route keeps a margin the body does not.
fn next_aim(
    ob: &Obstacles,
    route: &mut Route,
    from: Vec2,
    dest: Vec2,
    radius: Fixed,
) -> (Vec2, bool) {
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
    visible_along(ob, from, points, rules::units(rules::LOCAL_REACH), radius)
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
