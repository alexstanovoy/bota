//! Choosing what to attack, and holding to it: one rule for everything that
//! fights of its own accord.

use bota_core::{PriorityOrder, class_rank, isqrt64, kind_def, rules};
use bota_proto::{Fixed, Target, UnitKind};

use crate::game::{Candidates, Entity, UnitOrder, World};

use crate::profile::Phase;

impl World {
    /// Where a candidate sits by what it is doing. Lower is taken first.
    ///
    /// A hero laying into this side counts for no more than a plain unit. A
    /// hero doing nothing to this side comes after both, and one putting out
    /// its own comes last.
    pub fn threat_priority(&self, seeker: Entity, candidate: Entity) -> u8 {
        let Some(side) = self.team.get(seeker).copied() else {
            return 1;
        };
        if self.kind.get(candidate) != Some(&UnitKind::Hero) {
            return 1;
        }
        // What it is doing is read off its order, what it is holding, and the
        // spell it is in the middle of: a spell at this side is as plain as a
        // swing at it.
        let casting_at = self
            .pending_cast(candidate)
            .and_then(|pending| match pending.target() {
                Target::Unit(target) => self.of_wire(target),
                _ => None,
            });
        let struck = match self.orders.get(candidate).map(|o| o.current) {
            Some(UnitOrder::Attack { target, .. }) => Some(target),
            _ => casting_at.or_else(|| self.target_of(candidate)),
        };
        let Some(at_side) = struck.and_then(|t| self.team.get(t).copied()) else {
            return 2;
        };
        if at_side == side {
            1
        } else if Some(&at_side) == self.team.get(candidate) {
            3
        } else {
            2
        }
    }

    /// The best thing this entity may take on within a reach.
    ///
    /// Ranked by what it is, then by what it is doing, then by how far off it
    /// stands. A dead tie goes by a rank the match draws for each pair of
    /// seeker and candidate: the same answer on every run and every tick,
    /// and no side, spawned into the lower slots, taken first by standing.
    pub fn best_valid_in_range(
        &self,
        seeker: Entity,
        reach: Fixed,
        candidates: &Candidates,
    ) -> Option<Entity> {
        let order = self.priority_of(seeker);
        let at = self.pos(seeker)?;
        let _profile = self.scope(Phase::TargetQuery);
        let mut best: Option<((u8, u8, i64), Entity)> = None;
        for candidate in self.near(seeker, at, reach, candidates) {
            if candidate == seeker || !self.reachable(seeker, reach, candidate) {
                continue;
            }
            let far = self
                .transform
                .get(candidate)
                .map_or(i64::MAX, |t| isqrt64(at.distance_squared(t.pos)));
            let key = (
                self.class_priority(candidate, order),
                self.threat_priority(seeker, candidate),
                far,
            );
            // The rank is drawn only for a dead tie, which is rare.
            let better = best.is_none_or(|(had, held)| match key.cmp(&had) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Equal => {
                    (self.tie_rank(seeker, candidate), candidate)
                        < (self.tie_rank(seeker, held), held)
                }
                std::cmp::Ordering::Greater => false,
            });
            if better {
                best = Some((key, candidate));
            }
        }
        best.map(|(_, candidate)| candidate)
    }

    /// What this entity should be set on this tick.
    ///
    /// Nothing held, or something held that may no longer be taken on, means
    /// taking the best within acquisition; one that may not is kept only
    /// while its hold lasts and there is nothing else. Something held is kept
    /// while its hold lasts, and one held that is merely out of sight is
    /// chased into the fog for as long. Past that, one within attack range is kept
    /// unless something of a better class is within attack range too; one
    /// out of it gives way to the best within acquisition, and is kept only
    /// when there is none and its chase has not run out.
    pub fn select_target(&self, seeker: Entity, candidates: &Candidates) -> Option<Entity> {
        let stats = self.stats.get(seeker);
        let acquisition = stats.map_or(Fixed::ZERO, |stats| stats.acquisition);
        let reach = stats.map_or(Fixed::ZERO, |stats| stats.attack_range);
        let ai = self.lane_ai.get(seeker).copied();
        let keeping = ai.is_some_and(|ai| self.tick < ai.keep_until);
        let Some(held) = self.target_of(seeker) else {
            return self.best_valid_in_range(seeker, acquisition, candidates);
        };
        if keeping && self.hostile_unseen(seeker, held) {
            return Some(held);
        }
        if !self.hostile(seeker, held) {
            return self
                .best_valid_in_range(seeker, acquisition, candidates)
                .or_else(|| keeping.then_some(held));
        }
        if keeping {
            return Some(held);
        }
        if self.reachable(seeker, reach, held) {
            let order = self.priority_of(seeker);
            if self.class_priority(held, order) == 0 {
                return Some(held);
            }
            let best = self.best_valid_in_range(seeker, reach, candidates);
            let better = best.is_some_and(|best| {
                self.class_priority(best, order) < self.class_priority(held, order)
            });
            return if better { best } else { Some(held) };
        }
        if ai.is_some_and(|ai| self.tick >= ai.chase_until) {
            return self.best_valid_in_range(seeker, acquisition, candidates);
        }
        self.best_valid_in_range(seeker, acquisition, candidates)
            .or(Some(held))
    }

    /// Runs one tick of choosing for everything that fights of its own accord.
    ///
    /// What is taken on depends on the order in hand: one told to walk
    /// somewhere, to follow or to stand takes on nothing, one told to attack
    /// takes on that and nothing else, and one left to itself chooses, unless
    /// what it wears disables its auto-attack. Every order cooldown runs one
    /// tick down here.
    pub fn tick_targeting(&mut self) {
        let mut candidates = std::mem::take(&mut self.candidates);
        self.lay_candidates(&mut candidates);
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            if let Some(orders) = self.orders.get_mut(entity) {
                orders.cooldown = orders.cooldown.saturating_sub(1);
                orders.release_cooldown = orders.release_cooldown.saturating_sub(1);
            }
            if !self.can_attack(entity) {
                continue;
            }
            // Channelling or feared, it takes on nothing at all; neither does
            // a body still walking in to make a cast.
            if self.is_channelling(entity) || self.feared(entity) || self.cast_out_of_reach(entity)
            {
                self.target.remove(entity);
                continue;
            }
            match self.orders.get(entity).map(|o| o.current) {
                Some(UnitOrder::Attack { target, .. }) => {
                    if self.may_attack_on_order(entity, target) {
                        self.set_target(entity, target);
                    } else {
                        self.target.remove(entity);
                    }
                    continue;
                }
                Some(UnitOrder::Move { .. } | UnitOrder::Stand | UnitOrder::Follow { .. }) => {
                    self.target.remove(entity);
                    continue;
                }
                // Left to itself, what will not auto-attack takes on nobody.
                Some(UnitOrder::Idle) if self.auto_attack_disabled(entity) => {
                    self.target.remove(entity);
                    continue;
                }
                _ => {}
            }
            match self.chosen_target(entity, &candidates) {
                Some(found) => self.set_target(entity, found),
                None => {
                    self.target.remove(entity);
                }
            }
            self.mark_chase(entity);
        }
        self.recycle_entity_snapshot(entities);
        self.candidates = candidates;
    }

    /// Keeps every order aimed at a unit honest about what its side sees.
    ///
    /// For a target still seen, the last seen spot moves with it. For one
    /// its side has lost, the order degrades toward that spot (an attack
    /// into an attack-move, a follow into a walk) and the target is dropped.
    /// Runs on the sight laid out earlier in the same tick.
    pub fn tend_attack_orders(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let (target, last_seen, fighting) = match self.orders.get(entity).map(|o| o.current) {
                Some(UnitOrder::Attack { target, last_seen }) => (target, last_seen, true),
                Some(UnitOrder::Follow { target, last_seen }) => (target, last_seen, false),
                _ => continue,
            };
            // What has fallen is settled where the burying is.
            if !self.alive(target) {
                continue;
            }
            if self.can_see_of(entity, target) {
                if let Some(at) = self.pos(target) {
                    let kept = if fighting {
                        UnitOrder::Attack {
                            target,
                            last_seen: at,
                        }
                    } else {
                        UnitOrder::Follow {
                            target,
                            last_seen: at,
                        }
                    };
                    self.set_order(entity, kept);
                }
            } else {
                let degraded = if fighting {
                    UnitOrder::AttackMove { pos: last_seen }
                } else {
                    UnitOrder::Move { pos: last_seen }
                };
                self.set_order(entity, degraded);
                self.target.remove(entity);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// What this entity should be set on, everything else it carries taken
    /// into account.
    ///
    /// A neutral walking home, asleep or disarmed takes nothing on, and one whose camp
    /// was struck takes whoever struck it. A creep just roused by an attack
    /// order takes whoever it was roused at, unless the order was aimed at one
    /// of the orderer's own, in which case the orderer goes last.
    fn chosen_target(&mut self, entity: Entity, candidates: &Candidates) -> Option<Entity> {
        if self.neutral_ai.get(entity).is_some_and(|ai| ai.going_home) {
            return None;
        }
        // A disarmed neutral cannot be aggroed.
        if self.neutral.get(entity).is_some() && self.disarmed(entity) {
            return None;
        }
        if let Some(mut ai) = self.neutral_ai.get(entity).copied()
            && let Some(by) = ai.roused_by.take()
        {
            self.neutral_ai.insert(entity, ai);
            if self.alive(by) && self.team.get(by) != self.team.get(entity) {
                return Some(by);
            }
        }
        // Asleep, it takes nothing on however near it stands.
        if self.neutral_ai.get(entity).is_some_and(|ai| !ai.awake) {
            return None;
        }
        if let Some(mut ai) = self.lane_ai.get(entity).copied()
            && let Some(orderer) = ai.roused_by.take()
        {
            let reach = self
                .stats
                .get(entity)
                .map_or(Fixed::ZERO, |stats| stats.acquisition);
            let order = self.priority_of(entity);
            let taken = if ai.roused_at_own {
                self.acquire_demoting(entity, reach, order, Some(orderer), candidates)
            } else {
                Some(orderer)
            };
            if taken != Some(orderer) {
                // Nothing was handed over, so nothing is held either.
                ai.keep_until = 0;
            }
            self.lane_ai.insert(entity, ai);
            return taken;
        }
        self.select_target(entity, candidates)
    }

    /// Keeps a creep's marks up to date: how long the chase has left and
    /// where it last saw what it is after.
    fn mark_chase(&mut self, entity: Entity) {
        let Some(mut ai) = self.lane_ai.get(entity).copied() else {
            return;
        };
        let reach = self
            .stats
            .get(entity)
            .map_or(Fixed::ZERO, |stats| stats.attack_range);
        match self.target_of(entity) {
            Some(held) => {
                ai.last_seen = self.pos(held);
                if self.reachable(entity, reach, held) {
                    ai.chase_until = self.tick + rules::CREEP_CHASE_TICKS;
                }
            }
            None => {
                let radius = rules::units(rules::LANE_WAYPOINT_RADIUS);
                if let Some(at) = self.pos(entity)
                    && ai.last_seen.is_some_and(|spot| at.within(spot, radius))
                {
                    ai.last_seen = None;
                }
            }
        }
        self.lane_ai.insert(entity, ai);
    }

    /// Hands a creep the target an attack order pointed it at, and holds it
    /// there for a while.
    ///
    /// Ordering an attack at one of your own hands the creep nothing: it ranks
    /// everyone else first and takes the one who gave the order only when there
    /// is nobody else.
    ///
    /// Neither pulling nor letting go breaks a hold that is still running;
    /// how often a hero may pull is the orderer's wait, kept where the call
    /// goes out.
    pub fn provoke(&mut self, creep: Entity, orderer: Entity, at_own: bool) {
        let Some(mut ai) = self.lane_ai.get(creep).copied() else {
            return;
        };
        if self.tick < ai.keep_until {
            return;
        }
        ai.roused_by = Some(orderer);
        ai.roused_at_own = at_own;
        ai.keep_until = if at_own {
            0
        } else {
            self.tick + rules::ORDER_AGGRO_HOLD_TICKS
        };
        ai.chase_until = self.tick + rules::CREEP_CHASE_TICKS;
        self.lane_ai.insert(creep, ai);
    }

    /// Where a candidate sits by what it is. Lower is taken first.
    fn class_priority(&self, candidate: Entity, order: PriorityOrder) -> u8 {
        self.kind.get(candidate).map_or(u8::MAX, |kind| {
            class_rank(kind_def(*kind).target_class, order)
        })
    }
}
