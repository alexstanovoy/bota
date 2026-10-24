//! Walking: where an entity is going, the route and the plan that take it
//! there, and the step it takes this tick.

use bota_core::{
    clamp_to_map, facing_gap, facing_towards, isqrt64, move_towards, per_tick, point_along, rules,
    turn_towards,
};
use bota_proto::{Angle, Fixed, Target, Vec2};

use crate::game::{
    ActionPhase, ActionState, Blocked, Entity, LocalAsk, LocalScratch, Obstacles, Plan, Planner,
    Route, UnitOrder, World, plan_local, plan_radius,
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

    /// One entity's tick of walking, as its [`World::walk_goal`] says.
    fn walk_one(&mut self, entity: Entity, scratch: &mut LocalScratch) {
        match self.walk_goal(entity) {
            Goal::Flee(from) => self.flee(entity, from, scratch),
            Goal::Stand => self.stand(entity),
            Goal::Face(at) => {
                self.turn_to(entity, at);
                self.stand(entity);
            }
            Goal::Walk(dest, arrive) => self.walk_toward(entity, dest, arrive, scratch),
        }
    }

    /// What an entity's state has it do with its body this tick.
    ///
    /// Feared, it runs from whoever put the fear on. Held or channelling it
    /// stands, and so does a disarmed neutral; rooted, it stands too but
    /// comes round to what it is set on, to swing at it. Mid-swing, or
    /// casting, it stands and comes round to what it acts on. A cast aimed
    /// further off than it reaches walks it in. Past that, what it is set on
    /// decides: in reach it stands and comes round to it, out of reach it
    /// walks at it. Only with nothing to fight does it walk where it was
    /// told.
    fn walk_goal(&self, entity: Entity) -> Goal {
        if self.feared(entity) {
            return self.flees_from(entity).map_or(Goal::Stand, Goal::Flee);
        }
        if self.held(entity)
            || self.is_channelling(entity)
            || self.disarmed(entity) && self.neutral.get(entity).is_some()
        {
            return Goal::Stand;
        }
        let set_on = || self.target_of(entity).and_then(|on| self.pos(on));
        if self.rooted(entity) {
            return set_on().map_or(Goal::Stand, Goal::Face);
        }
        if let Some(face) = self.acting_on(entity) {
            return face
                .and_then(|on| self.pos(on))
                .map_or(Goal::Stand, Goal::Face);
        }
        if let Some(pending) = self.pending_cast(entity)
            && let Some(aim) = self.cast_spot(pending)
        {
            let reach = self.cast_reach(entity, pending);
            if reach > 0 {
                return Goal::Walk(aim, rules::units(reach));
            }
        }
        let holding = matches!(
            self.orders.get(entity).map(|o| o.current),
            Some(UnitOrder::Hold)
        );
        match self.closing_on(entity) {
            // Holding, it comes round to what it is set on but never leaves
            // the spot it was left on.
            Some((at, _)) if holding => Goal::Face(at),
            Some((at, reach)) => Goal::Walk(at, reach),
            None => self
                .orders
                .get(entity)
                .and_then(|o| destination(&o.current))
                .map_or(Goal::Stand, |dest| Goal::Walk(dest, Fixed::ZERO)),
        }
    }

    /// Whether an entity is walking this tick: its walk is not over
    /// ([`World::walk_unfinished`]) and it took its last step. A body that
    /// is not is stationary, and every walker's short pather lays its way
    /// round it (Liquipedia, *Pathfinding*): one that could not take its last
    /// step, pressed against others or the ground, stands there as much as
    /// one told to.
    pub(super) fn walking(&self, entity: Entity) -> bool {
        self.walk_unfinished(entity)
            && self
                .motion
                .get(entity)
                .is_none_or(|motion| motion.stalled == 0)
    }

    /// Whether an entity with its walk ahead of it stood this tick coming
    /// round onto its way (Liquipedia, *Turn Rate*), neither stepping nor
    /// stopped by anything it walked into since its last step.
    pub fn turning_on_the_spot(&self, entity: Entity) -> bool {
        self.walk_unfinished(entity)
            && self
                .motion
                .get(entity)
                .is_some_and(|motion| motion.delta == Vec2::ZERO && motion.stalled == 0)
    }

    /// Whether an entity's orders still walk it somewhere it has not got to:
    /// its goal is a walk it has neither finished, by reaching the spot, nor
    /// ended, by its route coming as near as it gets. A walker held up for
    /// now still has its walk ahead of it.
    pub fn walk_unfinished(&self, entity: Entity) -> bool {
        let Some(from) = self.pos(entity) else {
            return false;
        };
        match self.walk_goal(entity) {
            Goal::Flee(_) => true,
            Goal::Stand | Goal::Face(_) => false,
            Goal::Walk(dest, arrive) => {
                from != dest
                    && !(arrive > Fixed::ZERO && from.within(dest, arrive))
                    && !self
                        .route
                        .get(entity)
                        .is_some_and(|route| route.done && route.goal == Some(dest))
            }
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
                    Target::Unit(target) => self.of_wire(target),
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
        let (on, reach) = match (chosen, ordered) {
            (Some(on), _) => (on, self.attack_reach(entity, on)),
            (None, Some(on)) => (on, self.touching_distance(entity, on)),
            (None, None) => return None,
        };
        Some((self.pos(on)?, reach))
    }

    /// How near an attacker's centre has to come to a target's to strike
    /// it: its attack range and both bound radii.
    fn attack_reach(&self, attacker: Entity, target: Entity) -> Fixed {
        let range = self
            .stats
            .get(attacker)
            .map_or(Fixed::ZERO, |stats| stats.attack_range);
        range + self.hulls(attacker, target)
    }

    /// How near two bodies' centres are when the bodies touch: a hair
    /// further apart than the hulls themselves, so what stops here is not
    /// overlapping and is not eased away again by [`World::push_apart`].
    fn touching_distance(&self, one: Entity, other: Entity) -> Fixed {
        self.collision(one) + self.collision(other) + rules::units(rules::STEER_MARGIN)
    }

    /// An entity's collision size; zero for one without a body.
    pub(super) fn collision(&self, entity: Entity) -> Fixed {
        self.hull.get(entity).map_or(Fixed::ZERO, |h| h.collision)
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
            self.pos(entity),
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
    /// it, one tick's worth, as Dota walks every ground unit (Liquipedia,
    /// *Pathfinding*): the long pather's route keeps it round the ground and
    /// the trees, the short pather's plan along it round the stationary
    /// bodies about it, and a body that moves is met where it is
    /// ([`World::collide`]).
    ///
    /// Standing near enough already, it only comes round to face the spot.
    /// What flies goes straight.
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
        let collision = self.collision(entity);
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
            before_relay: None,
        };
        let goal = self
            .route
            .get(entity)
            .and_then(|route| route.goal)
            .unwrap_or(dest);
        let stalled = self.motion.get(entity).map_or(0, |motion| motion.stalled);
        // A plan is walked from where it was laid: a body put somewhere
        // else since, by a hook or a slide along a body, has no plan.
        if (plan_stale(self.plan.get(entity), from, step, goal, stalled, self.tick)
            || self.wait_over(entity))
            && self.relay_plan(&mut leg, goal, scratch)
        {
            return;
        }
        self.follow_plan(&leg);
    }

    /// A step a unit takes facing the way it walks, `heading`: it turns
    /// that way and sets off only once it faces within 11.5 degrees of it
    /// (Liquipedia, *Turn Rate*); under way it keeps walking while it comes
    /// round. The step lands on `to`: where it walks, or where a body it
    /// walks into leaves it, slid along that body's edge. A slide is the
    /// push of the bodies against each other, not a way the walker turns to:
    /// it keeps facing where it walks.
    fn step_heading(&mut self, entity: Entity, from: Vec2, heading: Vec2, to: Vec2) {
        let (Some(facing), Some(rate)) = (
            self.transform.get(entity).map(|t| t.facing),
            self.stats.get(entity).map(|stats| stats.turn_rate),
        ) else {
            return;
        };
        let wanted = facing_towards(from, heading);
        let facing = turn_towards(facing, wanted, rate);
        if let Some(transform) = self.transform.get_mut(entity) {
            transform.facing = facing;
        }
        let under_way = self.motion.get(entity).is_some_and(|m| m.still <= 1);
        let tolerance = if under_way {
            rules::TURN_TOLERANCE_BRADS
        } else {
            rules::ATTACK_ANGLE_BRADS
        };
        if facing_gap(facing, wanted) <= tolerance {
            self.moved(entity, from, to);
        }
    }

    /// A walker whose step a body cuts, or whose short pather finds no way
    /// through the bodies about it, walks on at `toward` into the body that
    /// blocks it: a moving body is no obstacle to the short pather, so the
    /// walker meets it, and its step keeps the part that runs along the
    /// body's edge ([`slide_along`]). A creep met head on is held; one met
    /// off centre slides past, the blocked creep that "will try to pass on
    /// your left/right side" (Gaming StackExchange q/65482). A body in front
    /// of the step ([`rules::UNIT_BLOCKED_COS_PERMILLE`]) blocks the walk,
    /// and the walker waits on it before its short pather lays its way round
    /// it ([`World::wait_on`]): a creep blocker stopping in front of a creep
    /// holds it so long. Approximation: Dota does not document how a step
    /// against a unit resolves; the slide keeps the hold a creep block needs.
    ///
    /// A step met dead on runs along no edge, and the walker stands this
    /// tick. Standing, it is a stationary unit ([`World::walking`]), which
    /// the other's short pather lays its way round, and the other likewise
    /// for it: two walking into each other dead on do not push for good.
    /// Against one walking its way, or a body stepped in front of it, it is
    /// held while that body keeps it there.
    ///
    /// Answers whether it took a step or came round for one; it stands when
    /// the slide is cut too.
    fn collide(&mut self, leg: &Leg, toward: Vec2) -> bool {
        let (entity, from, collision) = (leg.entity, leg.from, leg.collision);
        let next = move_towards(from, toward, leg.step);
        // A body in front of the step blocks the walk, walking or standing:
        // the walker stands and waits on it, and only then slides along it
        // or lays its way round it ([`World::wait_on`]). One that waits on
        // the walker in turn is not waited on: neither would ever move.
        if let Some((body, centre, _)) = self.stood_in_the_way(entity, from, next, collision)
            && within_cone(from, next, centre, rules::UNIT_BLOCKED_COS_PERMILLE)
            && !self.waits_on(body, entity)
            && self.wait_on(entity, body)
        {
            return false;
        }
        // Slid along one body into another, it slides along that one too,
        // and so on; met by one it slid along already, it is wedged.
        let mut met: Vec<Entity> = Vec::new();
        let mut to = Some(next);
        while let Some(step) = to
            && let Some((body, centre, radius)) =
                self.stood_in_the_way(entity, from, step, collision)
        {
            if met.contains(&body) {
                to = None;
                break;
            }
            met.push(body);
            to = slide_along(from, step, centre, collision + radius);
        }
        let Some(to) = to.filter(|to| {
            *to != from
                && self
                    .stood_in_the_way(entity, from, *to, collision)
                    .is_none()
                && self.clearance.capsule_clear(from, *to, collision)
        }) else {
            return false;
        };
        if let Some(plan) = self.plan.get_mut(entity) {
            plan.clear();
        }
        self.step_heading(entity, from, next, to);
        true
    }

    /// The body a step would walk into, where it stood when the tick began,
    /// and its collision size: the nearest to where the step begins of those
    /// the step ends inside and nearer their centre. Phased bodies are
    /// walked through.
    fn stood_in_the_way(
        &self,
        mover: Entity,
        from: Vec2,
        next: Vec2,
        mine: Fixed,
    ) -> Option<(Entity, Vec2, Fixed)> {
        let mut best: Option<(i64, Entity, Vec2, Fixed)> = None;
        self.bodies.near(next, mine, |body| {
            if body.entity == mover || self.phased(body.entity) {
                return;
            }
            let least = (mine + body.radius).squared_raw();
            let ahead = next.distance_squared(body.at);
            let near = from.distance_squared(body.at);
            if ahead >= least || ahead >= near {
                return;
            }
            if best.is_none_or(|(had, _, _, _)| near < had) {
                best = Some((near, body.entity, body.at, body.radius));
            }
        });
        best.map(|(_, entity, at, radius)| (entity, at, radius))
    }

    /// Where a walker stands and how far it walks this tick, or nothing
    /// when it does not walk: near enough already, it comes round to face
    /// the spot; unable to move, it stands; flying, it goes straight.
    fn walking_pace(&mut self, entity: Entity, dest: Vec2, arrive: Fixed) -> Option<(Vec2, Fixed)> {
        let from = self.pos(entity)?;
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

    /// Lays a walker's plan afresh along its route, round the stationary
    /// bodies about it, and answers whether that settles its walk this tick.
    fn relay_plan(&mut self, leg: &mut Leg, goal: Vec2, scratch: &mut LocalScratch) -> bool {
        let (entity, now) = (leg.entity, self.tick);
        let mut stationary = std::mem::take(&mut self.extra_scratch);
        stationary.clear();
        self.stationary_about(leg, &mut stationary);
        let crowded = !stationary.is_empty();
        let (mut steps, mut reached) = self.lay_plan(leg, &stationary, scratch);
        // Short of the aim with stationary bodies about, the route itself is
        // laid round them, as if they were structures: the short pather's
        // wall-tracing round what stands in the long pather's way.
        // A route that finds no way nearer the destination round them leads
        // nowhere, and the walker keeps the one it had.
        if !reached && crowded {
            let before = self.route.get(entity).cloned();
            let (aim, last) =
                self.lay_route(leg.entity, leg.from, leg.dest, leg.collision, &stationary);
            let nowhere = self
                .route
                .get(entity)
                .is_some_and(|route| route.corners.is_empty() && route.end == leg.from);
            if nowhere {
                if let (Some(slot), Some(before)) = (self.route.get_mut(entity), before) {
                    *slot = before;
                }
            } else {
                leg.try_relay(aim, last);
                let (round, reached_round) = self.lay_plan(leg, &stationary, scratch);
                if reached_round || round.len() > steps.len() {
                    steps = round;
                    reached = reached_round;
                } else {
                    leg.undo_relay();
                }
            }
        }
        self.extra_scratch = stationary;
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
        self.walk_nowhere(leg, reached);
        true
    }

    /// A walker whose fresh plan takes it nowhere: standing on a spot of
    /// its route already, at the end of its walk, or blocked for now.
    fn walk_nowhere(&mut self, leg: &Leg, reached: bool) {
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
        // walking into its way, that is the last word of the ground and the
        // stationary bodies about, which its short pather took in, and the
        // walk is over as near as it got; a body walking into its way it
        // walks into as far as that body lets it.
        let met = || {
            let next = move_towards(leg.from, leg.aim, leg.step);
            self.stood_in_the_way(entity, leg.from, next, leg.collision)
                .is_some_and(|(_, centre, radius)| !self.bodies.stands_still(centre, radius))
        };
        if leg.last && !met() {
            if let Some(route) = self.route.get_mut(entity) {
                route.done = true;
            }
            self.turn_to(entity, leg.dest);
            return;
        }
        self.walk_into(leg, leg.aim);
    }

    /// Takes the next step of a walker's plan, unless it turns onto the
    /// next stretch, or a body stands in the step: a body moving, which the
    /// short pather does not plan round, is met ([`World::collide`]).
    fn follow_plan(&mut self, leg: &Leg) {
        let (entity, from) = (leg.entity, leg.from);
        let Some(target) = self
            .plan
            .get(entity)
            .and_then(|plan| plan.steps.get(plan.at).copied())
        else {
            self.walk_into(leg, leg.aim);
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
        if !self.phased(entity) && self.body_in_the_way(entity, from, target).is_some() {
            self.walk_into(leg, target);
            return;
        }
        self.take_step(entity, from, target);
        if let Some(plan) = self.plan.get_mut(entity) {
            plan.at += 1;
        }
    }

    /// A walker that walks into what is in its way, at `toward`: as far as
    /// the body it meets lets it ([`World::collide`]), else it stands this
    /// tick wanting to move.
    fn walk_into(&mut self, leg: &Leg, toward: Vec2) {
        if !self.collide(leg, toward) {
            if let Some(plan) = self.plan.get_mut(leg.entity) {
                plan.clear();
            }
            self.stall(leg.entity, leg.dest);
        }
    }

    /// Lays the next stretch of a walk: the plan from where the walker
    /// stands to its aim, round the ground and the `stationary` bodies.
    /// Answers the plan and whether it gets to the aim.
    fn lay_plan(
        &self,
        leg: &Leg,
        stationary: &[(Vec2, Fixed)],
        scratch: &mut LocalScratch,
    ) -> (Vec<Vec2>, bool) {
        let _profile = self.scope(Phase::LocalPlan);
        let ob = Obstacles {
            field: &self.clearance,
            extra: stationary,
        };
        let ask = LocalAsk {
            from: leg.from,
            facing: self
                .transform
                .get(leg.entity)
                .map_or(Angle::default(), |t| t.facing),
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
        let _search_profile = self.scope(Phase::LocalSearch);
        plan_local(&ob, &ask, scratch)
    }

    /// The stationary bodies within a walker's plan's reach, where they
    /// stand, as circles its short pather lays its way round: a body that
    /// is not walking ([`World::walking`]) and can move at all (what never
    /// moves is ground already). A walker that is phased walks through all
    /// of them, and a phased body is walked through. A body that stands
    /// stays where it is, so one beyond what the plan's horizon and the
    /// walker's own room reach cannot touch the plan.
    fn stationary_about(&self, leg: &Leg, out: &mut Vec<(Vec2, Fixed)>) {
        if self.phased(leg.entity) {
            return;
        }
        let reach = Fixed {
            raw: leg
                .step
                .raw
                .saturating_mul(rules::LOCAL_HORIZON_TICKS as i32),
        } + plan_radius(leg.collision);
        let blocked = self
            .motion
            .get(leg.entity)
            .and_then(|motion| motion.blocked);
        self.bodies.near(leg.from, reach, |body| {
            if body.entity == leg.entity || body.fixed || self.phased(body.entity) {
                return;
            }
            if !body.stationary {
                return;
            }
            // The standing body it waits on, ahead on its way, is laid round
            // once the wait is over; one beside its way at once.
            if let Some(blocked) = blocked
                && blocked.by == body.entity
                && self.tick < blocked.until
                && within_cone(
                    leg.from,
                    leg.aim,
                    body.at,
                    rules::UNIT_BLOCKED_LOOK_AHEAD_COS_PERMILLE,
                )
            {
                return;
            }
            out.push((body.at, body.radius));
        });
    }

    /// Keeps an entity's route to a destination: laid afresh when the
    /// destination is new or has drifted, its last leg swung onto a
    /// destination that moved a little, its corners dropped as they are
    /// passed, and laid again from where the entity stands when the way
    /// to its next corner is shut.
    ///
    /// A route laid round stationary bodies (`extra`) keeps them in mind
    /// while they stand ([`Route::round`]): its corners are passed and its
    /// aim taken round them too, as the short pather keeps tracing a wall
    /// until its way is free. One that walks off is forgotten and the route
    /// laid again without it; a new destination forgets them all, the long
    /// pather knowing no units.
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
        let mut round = std::mem::take(&mut route.round);
        let traced = round.len();
        round.retain(|&(at, radius)| self.bodies.stands_still(at, radius));
        let left = round.len() < traced;
        for circle in extra {
            if !round.contains(circle) {
                round.push(*circle);
            }
        }
        let kept = {
            let ob = Obstacles {
                field: &self.clearance,
                extra: &round,
            };
            keeps_route(&ob, &mut route, from, dest, room)
        };
        if !kept {
            round.clear();
        }
        let ob = Obstacles {
            field: &self.clearance,
            extra: &round,
        };
        let fresh = !kept || left || !extra.is_empty();
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
        route.round = round;
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
        // A free step ends a wait on a body that blocked the walk.
        if let Some(motion) = self.motion.get_mut(entity) {
            motion.blocked = None;
        }
        let rate = self.stats.get(entity).map_or(0, |stats| stats.turn_rate);
        if let Some(transform) = self.transform.get_mut(entity) {
            transform.facing = turn_towards(transform.facing, facing_towards(from, to), rate);
        }
        self.moved(entity, from, to);
    }

    fn moved(&mut self, entity: Entity, from: Vec2, to: Vec2) {
        if let Some(transform) = self.transform.get_mut(entity) {
            transform.pos = to;
        }
        if let Some(motion) = self.motion.get_mut(entity) {
            motion.delta = to - from;
            motion.still = 0;
            motion.stalled = 0;
        }
    }

    /// A walker blocked by `body` waits on it, and answers whether it still
    /// waits this tick: the first wait when the block
    /// is new, the next once its way round that body was laid and the body
    /// cut it again, and the last every time after
    /// ([`rules::UNIT_BLOCKED_WAIT_TICKS`]). A free step ends the wait
    /// ([`World::take_step`]).
    fn wait_on(&mut self, entity: Entity, body: Entity) -> bool {
        let now = self.tick;
        let laid = self.plan.get(entity).map(|plan| plan.laid);
        let Some(motion) = self.motion.get_mut(entity) else {
            return false;
        };
        let wait = |tier: u8| rules::UNIT_BLOCKED_WAIT_TICKS[usize::from(tier)];
        motion.blocked = Some(match motion.blocked {
            Some(blocked)
                if blocked.by == body && laid.is_some_and(|laid| laid >= blocked.until) =>
            {
                let tier = (blocked.tier + 1).min(2);
                Blocked {
                    by: body,
                    until: now + wait(tier),
                    tier,
                }
            }
            Some(blocked) if blocked.by == body => blocked,
            _ => Blocked {
                by: body,
                until: now + wait(0),
                tier: 0,
            },
        });
        motion.blocked.is_some_and(|blocked| now < blocked.until)
    }

    /// Whether `entity` waits on `body` this tick.
    fn waits_on(&self, entity: Entity, body: Entity) -> bool {
        self.motion
            .get(entity)
            .and_then(|motion| motion.blocked)
            .is_some_and(|blocked| blocked.by == body && self.tick < blocked.until)
    }

    /// Whether a walker's wait on the body that blocks it is over and it has
    /// laid no way round that body since.
    fn wait_over(&self, entity: Entity) -> bool {
        let laid = self.plan.get(entity).map(|plan| plan.laid);
        self.motion
            .get(entity)
            .and_then(|motion| motion.blocked)
            .is_some_and(|blocked| {
                self.tick >= blocked.until && laid.is_none_or(|laid| laid < blocked.until)
            })
    }

    /// A tick stood wanting to move and unable to.
    fn stall(&mut self, entity: Entity, dest: Vec2) {
        self.turn_to(entity, dest);
        if let Some(motion) = self.motion.get_mut(entity) {
            motion.stalled = motion.stalled.saturating_add(1);
        }
    }

    /// What flies goes straight: closed ground and bodies are nothing to
    /// it. It turns first, like everything else.
    fn fly_toward(&mut self, entity: Entity, dest: Vec2, step: Fixed) {
        let (Some(at), Some(rate)) = (
            self.transform.get(entity).copied(),
            self.stats.get(entity).map(|stats| stats.turn_rate),
        ) else {
            return;
        };
        let from = at.pos;
        let wanted = facing_towards(from, dest);
        let facing = turn_towards(at.facing, wanted, rate);
        if let Some(transform) = self.transform.get_mut(entity) {
            transform.facing = facing;
        }
        if facing_gap(facing, wanted) <= rules::TURN_TOLERANCE_BRADS {
            let next = clamp_to_map(move_towards(from, dest, step));
            self.take_step(entity, from, next);
        }
    }

    /// Runs one entity straight away from another, a step a tick.
    fn flee(&mut self, entity: Entity, from: Entity, scratch: &mut LocalScratch) {
        let (Some(here), Some(there)) = (self.pos(entity), self.pos(from)) else {
            return;
        };
        if here == there {
            return;
        }
        let away = clamp_to_map(point_along(
            here,
            here + (here - there),
            Fixed::from_int(rules::FLEE_LOOKAHEAD),
        ));
        self.walk_toward(entity, away, Fixed::ZERO, scratch);
    }
}

/// One walker's tick of walking along its route.
struct Leg {
    entity: Entity,
    from: Vec2,
    dest: Vec2,
    /// How near the destination is near enough.
    arrive: Fixed,
    /// How far it walks in a tick.
    step: Fixed,
    collision: Fixed,
    /// The spot it walks at next.
    aim: Vec2,
    /// Whether that spot is the route's end.
    last: bool,
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

/// Whether `at` lies within the cone round the way from `from` to `toward`
/// whose cosine, in thousandths, is `cos_permille`.
fn within_cone(from: Vec2, toward: Vec2, at: Vec2, cos_permille: i64) -> bool {
    // In 1/256 units: any two points' offsets fit 25 bits, so every product
    // below stays well inside i128.
    let offset = |a: Fixed, b: Fixed| (i128::from(a.raw) - i128::from(b.raw)) >> 8;
    let way = (offset(toward.x, from.x), offset(toward.y, from.y));
    let to = (offset(at.x, from.x), offset(at.y, from.y));
    let dot = way.0 * to.0 + way.1 * to.1;
    if dot <= 0 {
        return false;
    }
    let lengths = (way.0 * way.0 + way.1 * way.1) * (to.0 * to.0 + to.1 * to.1);
    let cos = i128::from(cos_permille);
    // dot / (|way| |to|) >= cos / 1000, both sides squared.
    dot * dot * 1_000_000 >= cos * cos * lengths
}

/// Whether the laid plan still leads where the route goes and has enough left.
/// A plan that was never laid is stale.
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
        let leg = isqrt64(at.distance_squared(point));
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

/// Where a step from one spot to another ends against a circle it would
/// enter: the part of the step that runs across the line to the circle's
/// centre, set back onto the circle's edge. Nothing when no part does,
/// which is a step straight at the centre.
fn slide_along(from: Vec2, next: Vec2, centre: Vec2, reach: Fixed) -> Option<Vec2> {
    let (sx, sy) = (
        i128::from(next.x.raw) - i128::from(from.x.raw),
        i128::from(next.y.raw) - i128::from(from.y.raw),
    );
    let (nx, ny) = (
        i128::from(from.x.raw) - i128::from(centre.x.raw),
        i128::from(from.y.raw) - i128::from(centre.y.raw),
    );
    let norm = nx * nx + ny * ny;
    if norm == 0 {
        return None;
    }
    let inward = (sx * nx + sy * ny).min(0);
    let (tx, ty) = (sx - nx * inward / norm, sy - ny * inward / norm);
    if tx == 0 && ty == 0 {
        return None;
    }
    // Off the centre by the tangent stretch, then out to the edge.
    let (ox, oy) = (nx + tx, ny + ty);
    let off = i128::from(isqrt64(i64::try_from(ox * ox + oy * oy).ok()?));
    let edge = i128::from(reach.raw) + 1;
    let (ox, oy) = if off < edge && off > 0 {
        (ox * edge / off, oy * edge / off)
    } else {
        (ox, oy)
    };
    Some(Vec2 {
        x: Fixed {
            raw: i32::try_from(i128::from(centre.x.raw) + ox).ok()?,
        },
        y: Fixed {
            raw: i32::try_from(i128::from(centre.y.raw) + oy).ok()?,
        },
    })
}

/// What an entity's state has it do with its body this tick.
enum Goal {
    /// Run from this entity.
    Flee(Entity),
    /// Stand where it is.
    Stand,
    /// Stand and come round to a spot.
    Face(Vec2),
    /// Walk to a spot until within a reach of it.
    Walk(Vec2, Fixed),
}
