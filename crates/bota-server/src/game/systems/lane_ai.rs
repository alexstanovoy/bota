//! Choosing what to attack, and holding to it.
//!
//! One rule serves everything that fights of its own accord. A candidate is
//! ranked by what it is, then by what it is doing, then by how far off it
//! stands, and the first of that order wins. What is held is given up only
//! when it stops being worth holding: gone from sight, no longer something to
//! strike, or beaten by something better once the holder can reach it.

use bota_proto::{Fixed, UnitKind};

use crate::game::{Entity, PriorityOrder, UnitOrder, World, class_rank_of};
use crate::game::{isqrt64, rules};

impl World {
    /// Where a candidate sits by what it is doing. Lower is taken first.
    ///
    /// A hero laying into this side counts for no more than a plain unit;
    /// what pulls a creep onto a hero is the order that roused it, not the
    /// ranking. A hero doing nothing to this side comes after both, and one
    /// putting out its own comes last.
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
            .casting
            .get(candidate)
            .and_then(|cast| match cast.target {
                bota_proto::Target::Unit(target) => self.of_wire(target),
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

    /// Whether one entity may be taken on by another right now.
    pub fn valid_target(&self, seeker: Entity, candidate: Entity) -> bool {
        self.hostile(seeker, candidate)
    }

    /// The best thing this entity may take on within a reach.
    ///
    /// Ranked by what it is, then by what it is doing, then by how far off it
    /// stands. The entity itself breaks a dead tie, so the answer is the same
    /// on every run.
    pub fn best_valid_in_range(&self, seeker: Entity, reach: Fixed) -> Option<Entity> {
        let order = self.priority_of(seeker);
        let at = self.transform.get(seeker)?.pos;
        self.entities
            .iter()
            .filter(|candidate| *candidate != seeker && self.valid_target(seeker, *candidate))
            .filter(|candidate| self.reachable(seeker, reach, *candidate))
            .min_by_key(|candidate| {
                let far = self
                    .transform
                    .get(*candidate)
                    .map_or(i64::MAX, |t| isqrt64(at.distance_squared(t.pos)));
                (
                    self.class_priority(*candidate, order),
                    self.threat_priority(seeker, *candidate),
                    far,
                    *candidate,
                )
            })
    }

    /// What this entity should be set on this tick.
    ///
    /// Nothing held means taking the best in reach of acquisition. Something
    /// held is kept while its hold lasts; past that only a better class in
    /// reach is worth turning to, and a chase that has run its course is given
    /// up for whatever else there is.
    pub fn select_target(&self, seeker: Entity) -> Option<Entity> {
        let acquisition = self
            .stats
            .get(seeker)
            .map_or(Fixed::ZERO, |stats| stats.acquisition);
        let reach = self
            .stats
            .get(seeker)
            .map_or(Fixed::ZERO, |stats| stats.attack_range);
        let ai = self.lane_ai.get(seeker).copied();
        let keeping = ai.is_some_and(|ai| self.tick < ai.keep_until);
        let Some(held) = self.target_of(seeker) else {
            return self.best_valid_in_range(seeker, acquisition);
        };
        if !self.valid_target(seeker, held) {
            return self
                .best_valid_in_range(seeker, acquisition)
                .or_else(|| keeping.then_some(held));
        }
        if keeping {
            return Some(held);
        }
        if self.reachable(seeker, reach, held) {
            // Close enough to strike: only a better class is worth turning to,
            // and only one already in reach.
            let order = self.priority_of(seeker);
            let best = self.best_valid_in_range(seeker, reach);
            let better = best.is_some_and(|best| {
                self.class_priority(best, order) < self.class_priority(held, order)
            });
            return if better { best } else { Some(held) };
        }
        if ai.is_some_and(|ai| self.tick >= ai.chase_until) {
            return self.best_valid_in_range(seeker, acquisition);
        }
        self.best_valid_in_range(seeker, acquisition).or(Some(held))
    }

    /// Runs one tick of choosing for everything that fights of its own accord.
    ///
    /// What is taken on depends on the order in hand: one told to walk
    /// somewhere or to stand takes on nothing, one told to attack takes on that
    /// and nothing else, and one left to itself chooses.
    pub fn tick_targeting(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            if let Some(orders) = self.orders.get_mut(entity) {
                orders.cooldown = orders.cooldown.saturating_sub(1);
            }
            if self.attacking.get(entity).is_none() {
                continue;
            }
            // Channelling, it takes on nothing at all; neither does a body
            // with a cast still waiting to be made.
            if self.is_channelling(entity) || self.casting.get(entity).is_some() {
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
                _ => {}
            }
            match self.chosen_target(entity) {
                Some(found) => self.set_target(entity, found),
                None => {
                    self.target.remove(entity);
                }
            }
            self.mark_chase(entity);
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Keeps every order aimed at a unit honest about what its side sees.
    ///
    /// For a target still seen, the last seen spot moves with it. For one
    /// its side has lost, the order degrades toward that spot — an attack
    /// into an attack-move, a follow into a walk — so the hidden body is
    /// not tracked. Runs on sight freshly laid out, which is what catches
    /// a body the tick it slips away.
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
                if let Some(at) = self.transform.get(target).map(|t| t.pos) {
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
    /// A neutral walking home takes nothing on, and one whose camp was struck
    /// takes whoever struck it. A creep just roused by an
    /// attack order takes whoever it was roused at, unless the order was aimed
    /// at one of the orderer's own, in which case the orderer goes last.
    fn chosen_target(&mut self, entity: Entity) -> Option<Entity> {
        if self.neutral_ai.get(entity).is_some_and(|ai| ai.going_home) {
            return None;
        }
        // A camp struck answers as one, whoever of it was struck, and answers
        // to a blow it never saw thrown.
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
                self.acquire_demoting(entity, reach, order, Some(orderer))
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
        self.select_target(entity)
    }

    /// Keeps a creep's marks up to date: how long the chase has left, where it
    /// last saw what it is after, and where it left its route.
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
                if ai.anchor.is_none() {
                    ai.anchor = self.transform.get(entity).map(|t| t.pos);
                }
                ai.last_seen = self.transform.get(held).map(|t| t.pos);
                if self.reachable(entity, reach, held) {
                    ai.chase_until = self.tick + rules::CREEP_CHASE_TICKS;
                }
            }
            None => {
                let radius = rules::units(rules::LANE_WAYPOINT_RADIUS);
                if let Some(at) = self.transform.get(entity).map(|t| t.pos) {
                    if ai.last_seen.is_some_and(|spot| at.within(spot, radius)) {
                        ai.last_seen = None;
                    }
                    if ai.anchor.is_some_and(|spot| at.within(spot, radius)) {
                        ai.anchor = None;
                    }
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
    /// One creep is pulled at most once every
    /// [`rules::ORDER_AGGRO_COOLDOWN_TICKS`]; orders in between pass it by.
    /// Being let go waits on nothing, but neither pulling nor letting go
    /// breaks a hold that is still running.
    pub fn provoke(&mut self, creep: Entity, orderer: Entity, at_own: bool) {
        let Some(mut ai) = self.lane_ai.get(creep).copied() else {
            return;
        };
        // A hold is not broken by an order of any kind: what pulled the creep
        // keeps it for its whole span.
        if self.tick < ai.keep_until {
            return;
        }
        // Pointing at one of your own lets go rather than pulls, so it waits
        // on nothing and spends nothing.
        if !at_own {
            if self
                .orders
                .get(creep)
                .is_some_and(|orders| orders.cooldown > 0)
            {
                return;
            }
            if let Some(orders) = self.orders.get_mut(creep) {
                orders.cooldown = rules::ORDER_AGGRO_COOLDOWN_TICKS;
            }
        }
        if ai.anchor.is_none() {
            ai.anchor = self.transform.get(creep).map(|t| t.pos);
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
        self.kind
            .get(candidate)
            .map_or(u8::MAX, |kind| class_rank_of(*kind, order))
    }
}
