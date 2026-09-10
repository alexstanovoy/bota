//! Picking a target, and clearing away what has fallen.

use bota_proto::{EventKind, Fixed, Team, UnitKind};

use crate::game::{Entity, MAP2_ID, MAP2_TICK_CAP, UnitOrder, World, is_structure, wire_id};
use crate::game::{Event, EventVisibility};

impl World {
    /// Each entity that can attack takes the best hostile in reach of its
    /// acquisition, and keeps it while it lives and stays in range.
    ///
    /// Whether an entity is still standing.
    pub fn alive(&self, entity: Entity) -> bool {
        self.entities.contains(entity)
            && self.health.get(entity).is_some_and(|h| h.hp > Fixed::ZERO)
    }

    /// Whether an attacker reaches its target, edge to edge.
    pub fn in_reach(&self, attacker: Entity, target: Entity) -> bool {
        let (Some(at), Some(their_at), Some(stats)) = (
            self.transform.get(attacker),
            self.transform.get(target),
            self.stats.get(attacker),
        ) else {
            return false;
        };
        let hulls = self.hull.get(attacker).map_or(Fixed::ZERO, |h| h.radius)
            + self.hull.get(target).map_or(Fixed::ZERO, |h| h.radius);
        at.pos.within(their_at.pos, stats.attack_range + hulls)
    }

    /// Turns every swing that came due into a hit or a missile in the air.
    ///
    /// The cycle itself is worked out before this, by [`attacking_system`];
    /// putting a missile in the world is a change of who exists, so it lives
    /// here.
    ///
    /// [`attacking_system`]: crate::game::attacking_system
    /// Tells each side what it was near enough to feel.
    pub fn tell_of(&self, felt: &[crate::game::Landed], events: &mut Vec<Event>) {
        for blow in felt {
            events.push(Event {
                kind: EventKind::Damaged {
                    source: blow.source.map(wire_id),
                    target: wire_id(blow.target),
                    amount: blow.amount,
                    kind: blow.kind,
                    crit: blow.crit,
                },
                visible_to: self.who_may_know(blow.at, blow.side),
            });
        }
    }

    /// Hands on the fights and follows aimed at a fallen entity.
    ///
    /// An attack order degrades to attack-moving at the spot the target was
    /// last seen, so the fight carries on with whatever acquisition finds
    /// there. A follow ends where the one followed fell.
    fn carry_fights_on(&mut self, fallen: Entity) {
        for follower in self.entities.iter().collect::<Vec<_>>() {
            match self.orders.get(follower).map(|o| o.current) {
                Some(UnitOrder::Follow { target, last_seen }) if target == fallen => {
                    self.set_order(follower, UnitOrder::Move { pos: last_seen });
                }
                Some(UnitOrder::Attack { target, last_seen }) if target == fallen => {
                    self.set_order(follower, UnitOrder::AttackMove { pos: last_seen });
                }
                _ => {}
            }
        }
    }

    /// Clears away what has fallen and tells who may know.
    pub fn bury(&mut self, fallen: Vec<(Entity, Option<Entity>)>, events: &mut Vec<Event>) {
        let mut structure_fell = false;
        let mut tower_losses = [false; 2];
        for (entity, killer) in fallen {
            if !self.entities.contains(entity) {
                continue;
            }
            if let Some((kind, side)) = self.bury_entity(entity, killer, events) {
                structure_fell = true;
                match (kind, side) {
                    (UnitKind::Tower, Team::Radiant) => tower_losses[0] = true,
                    (UnitKind::Tower, Team::Dire) => tower_losses[1] = true,
                    _ => {}
                }
            }
        }
        if structure_fell {
            self.lay_passability();
        }
        self.finish_map2_tick(tower_losses);
    }

    fn bury_entity(
        &mut self,
        entity: Entity,
        killer: Option<Entity>,
        events: &mut Vec<Event>,
    ) -> Option<(UnitKind, Team)> {
        assert!(self.entities.contains(entity));
        self.carry_fights_on(entity);
        self.feed_flesh_heaps(entity);
        self.feed_souls(entity, killer);
        let kind = self.kind.get(entity).copied();
        let side = self.team.get(entity).copied();
        let denied = killer
            .and_then(|k| self.team.get(k).copied())
            .is_some_and(|theirs| Some(theirs) == side);
        let paid = self.pay_for(entity, killer, events);
        let at = self
            .transform
            .get(entity)
            .map_or(bota_proto::Vec2::ZERO, |t| t.pos);
        events.push(Event {
            kind: EventKind::Died {
                unit: wire_id(entity),
                killer: killer.map(wire_id),
                denied,
                gold: paid,
            },
            visible_to: self.who_may_know(at, side.unwrap_or(Team::Neutral)),
        });
        let structure = kind.zip(side).filter(|(kind, _)| is_structure(*kind));
        if let Some((kind, side)) = structure {
            events.push(Event {
                kind: EventKind::StructureDestroyed {
                    unit: wire_id(entity),
                    team: side,
                },
                visible_to: EventVisibility::Everyone,
            });
            if self.map.id != MAP2_ID
                && self.winner.is_none()
                && (kind == UnitKind::Ancient
                    || (self.map.tower_ends_it && kind == UnitKind::Tower))
            {
                self.winner = Some(other_side(side));
            }
        }
        self.bury_seat(entity);
        if let Some(index) = killer.and_then(|k| self.seats.iter().position(|s| s.unit == Some(k)))
            && kind == Some(UnitKind::Hero)
            && !denied
        {
            self.seats[index].kills += 1;
        }
        self.despawn(entity);
        assert!(!self.entities.contains(entity));
        structure
    }

    fn bury_seat(&mut self, entity: Entity) {
        let mut fallen_hero_team = None;
        for index in 0..self.seats.len() {
            if self.seats[index].unit == Some(entity) {
                let level = self.seats[index].level;
                let kept = crate::game::Kept {
                    book: self.abilities.remove(entity).unwrap_or_default(),
                    bag: self.inventory.remove(entity).unwrap_or_default(),
                    stacks: self.stacks.remove(entity).unwrap_or_default(),
                };
                self.seats[index].unit = None;
                self.seats[index].kept = Some(kept);
                self.seats[index].deaths += 1;
                self.seats[index].respawn_left = World::respawn_wait(level);
                fallen_hero_team = Some(self.seats[index].team);
            }
            if self.seats[index].courier == Some(entity) {
                self.seats[index].courier_kept = self.inventory.remove(entity);
            }
        }
        if let Some(team) = fallen_hero_team
            && self.map.id != MAP2_ID
            && self.map.death_limit > 0
            && self.winner.is_none()
        {
            let deaths = self
                .seats
                .iter()
                .filter(|seat| seat.team == team)
                .map(|seat| u32::from(seat.deaths))
                .sum::<u32>();
            if deaths >= u32::from(self.map.death_limit) {
                self.winner = Some(other_side(team));
            }
        }
    }

    fn finish_map2_tick(&mut self, tower_losses: [bool; 2]) {
        if self.map.id != MAP2_ID || self.winner.is_some() {
            return;
        }
        let deaths = |team| {
            self.seats
                .iter()
                .filter(|seat| seat.team == team)
                .fold(0u16, |total, seat| total.saturating_add(seat.deaths))
        };
        let lost = [
            (self.map.tower_ends_it && tower_losses[0])
                || (self.map.death_limit > 0 && deaths(Team::Radiant) >= self.map.death_limit),
            (self.map.tower_ends_it && tower_losses[1])
                || (self.map.death_limit > 0 && deaths(Team::Dire) >= self.map.death_limit),
        ];
        self.winner = match (self.tick >= MAP2_TICK_CAP, lost) {
            (true, _) | (false, [true, true]) => Some(Team::Neutral),
            (false, [true, false]) => Some(Team::Dire),
            (false, [false, true]) => Some(Team::Radiant),
            (false, [false, false]) => None,
        };
    }

    pub(crate) fn map2_finished(&mut self) -> bool {
        if self.map.id != MAP2_ID {
            return false;
        }
        if self.tick >= MAP2_TICK_CAP {
            self.winner.get_or_insert(Team::Neutral);
        }
        self.winner.is_some()
    }
}

/// The side that is not this one.
fn other_side(team: Team) -> Team {
    match team {
        Team::Radiant => Team::Dire,
        Team::Dire => Team::Radiant,
        Team::Neutral => Team::Neutral,
    }
}
