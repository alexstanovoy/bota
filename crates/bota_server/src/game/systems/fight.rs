//! Who stands and who is in reach, telling of blows, and clearing away what
//! has fallen.

use bota_core::{MAP2_ID, MAP2_TICK_CAP, effect_def, kind_def, respawn_wait};
use bota_proto::{EventKind, Fixed, Team, UnitKind, Vec2};

use crate::game::{
    Entity, Event, EventVisibility, ItemStack, Kept, Landed, Missed, Modifiers, UnitOrder, World,
    wire_id,
};

impl World {
    /// Whether an entity is still standing.
    pub fn alive(&self, entity: Entity) -> bool {
        self.entities.contains(entity)
            && self.health.get(entity).is_some_and(|h| h.hp > Fixed::ZERO)
    }

    pub(crate) fn pos(&self, entity: Entity) -> Option<Vec2> {
        self.transform.get(entity).map(|t| t.pos)
    }

    /// Whether an attacker reaches its target, edge to edge.
    pub fn in_reach(&self, attacker: Entity, target: Entity) -> bool {
        self.stats
            .get(attacker)
            .is_some_and(|stats| self.within_reach(attacker, target, stats.attack_range))
    }

    /// Tells each side of the blows on what it sees, and on its own; who
    /// dealt one only to a side that sees the dealer.
    pub fn tell_of(&self, felt: &[Landed], events: &mut Vec<Event>) {
        for blow in felt {
            events.push(Event {
                kind: EventKind::Damaged {
                    source: blow.source.map(wire_id),
                    target: wire_id(blow.target),
                    amount: blow.amount,
                    kind: blow.kind,
                    inflictor: blow.inflictor,
                },
                visible_to: self.who_may_know(blow.target, None),
                gold_hidden_from: None,
                source_told_to: self.who_may_name(blow.source),
            });
        }
    }

    /// Tells each side of the attacks it sees miss, on the terms of
    /// [`World::tell_of`].
    pub fn tell_of_misses(&self, missed: &[Missed], events: &mut Vec<Event>) {
        for miss in missed {
            events.push(Event {
                kind: EventKind::Missed {
                    source: miss.source.map(wire_id),
                    target: wire_id(miss.target),
                },
                visible_to: self.who_may_know(miss.target, None),
                gold_hidden_from: None,
                source_told_to: self.who_may_name(miss.source),
            });
        }
    }

    /// Hands on the fights and follows aimed at a fallen entity.
    ///
    /// An attack order at it becomes an attack-move to the spot it was last
    /// seen; a follow becomes a walk to that spot. A follower with orders
    /// queued is done with it instead, and takes up the next.
    fn carry_fights_on(&mut self, fallen: Entity) {
        for follower in self.entities.iter().collect::<Vec<_>>() {
            let queued = self.has_queued(follower);
            match self.orders.get(follower).map(|o| o.current) {
                Some(UnitOrder::Follow { target, .. } | UnitOrder::Attack { target, .. })
                    if target == fallen && queued =>
                {
                    self.set_order(follower, UnitOrder::Idle);
                }
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
        // Who learns of each fall is worked out before anything is cleared
        // away: what falls beside another still saw it fall.
        let told: Vec<Told> = fallen
            .iter()
            .map(|&(entity, killer)| self.told_of_fall(entity, killer))
            .collect();
        let mut structure_fell = false;
        let mut tower_losses = [false; 2];
        for ((entity, killer), told) in fallen.into_iter().zip(told) {
            if !self.entities.contains(entity) {
                continue;
            }
            self.refresh_glyph_for(entity);
            if let Some((kind, side)) = self.bury_entity(entity, killer, told, events) {
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

    /// Who is told of a fall, and who is told who brought it down: both
    /// sides of what Dota announces, otherwise the side that fell, the side
    /// that brought it down and whoever sees it fall.
    ///
    /// An illusion's fall is no hero's: Dota's feed tells none, so it is
    /// told as any unit's is. The feed names a killer only when a hero (a
    /// player) struck last; of any other it names the side alone, which
    /// the fall's own side already knows.
    fn told_of_fall(&self, fallen: Entity, killer: Option<Entity>) -> Told {
        let def = self
            .kind
            .get(fallen)
            .filter(|_| !self.illusion.contains(fallen))
            .map(|kind| kind_def(*kind));
        let killer_side = killer.and_then(|killer| self.team.get(killer).copied());
        Told {
            fall: if def.is_some_and(|def| def.death_announced) {
                EventVisibility::Everyone
            } else {
                self.who_may_know(fallen, killer_side)
            },
            killer: if def.is_some_and(|def| def.killer_announced)
                && killer.is_some_and(|killer| self.is_real_hero(killer))
            {
                EventVisibility::Everyone
            } else {
                self.who_may_name(killer)
            },
        }
    }

    fn bury_entity(
        &mut self,
        entity: Entity,
        killer: Option<Entity>,
        told: Told,
        events: &mut Vec<Event>,
    ) -> Option<(UnitKind, Team)> {
        assert!(self.entities.contains(entity));
        self.cancel_action(entity);
        self.rot_goes_out(entity);
        self.carry_fights_on(entity);
        self.death_throes(entity);
        self.feed_flesh_heaps(entity);
        self.feed_souls(entity, killer);
        let kind = self.kind.get(entity).copied();
        let side = self.team.get(entity).copied();
        let denied = killer
            .and_then(|k| self.team.get(k).copied())
            .is_some_and(|theirs| Some(theirs) == side);
        let paid = self.pay_for(entity, killer, events);
        events.push(Event {
            kind: EventKind::Died {
                unit: wire_id(entity),
                killer: killer.map(wire_id),
                denied,
                gold: paid,
            },
            visible_to: told.fall,
            gold_hidden_from: killer
                .filter(|&k| kind != Some(UnitKind::Hero) && self.seat_of(k).is_some())
                .and_then(|k| self.team.get(k).copied())
                .map(other_side),
            source_told_to: told.killer,
        });
        let structure = kind.zip(side).filter(|(kind, _)| kind_def(*kind).structure);
        if let Some((kind, side)) = structure {
            events.push(Event {
                kind: EventKind::StructureDestroyed {
                    unit: wire_id(entity),
                    team: side,
                },
                visible_to: EventVisibility::Everyone,
                gold_hidden_from: None,
                source_told_to: EventVisibility::Everyone,
            });
            if self.map.id != MAP2_ID
                && self.winner.is_none()
                && (kind == UnitKind::Ancient
                    || (self.map.tower_ends_it && kind == UnitKind::Tower))
            {
                self.winner = Some(other_side(side));
            }
        }
        // Dota hands a hero a scroll for every death but one it dealt itself
        // or one neutral creeps are credited with.
        let scroll_due = killer.is_some_and(|killer| {
            killer != entity
                && self
                    .team
                    .get(killer)
                    .is_some_and(|side| *side != Team::Neutral)
        });
        self.bury_seat(entity, scroll_due);
        if let Some(index) = killer.and_then(|k| self.seat_of(k))
            && kind == Some(UnitKind::Hero)
            && !denied
        {
            self.seats[index].kills += 1;
        }
        self.despawn(entity);
        assert!(!self.entities.contains(entity));
        structure
    }

    /// What a fallen hero keeps of the modifiers on it until it stands again:
    /// those Dota does not take off on death.
    fn kept_through_death(&self, entity: Entity) -> Modifiers {
        Modifiers(
            self.modifiers
                .get(entity)
                .into_iter()
                .flat_map(|on_it| on_it.active())
                .filter(|held| {
                    effect_def(held.kind.effect_id()).is_some_and(|def| def.kept_through_death)
                })
                .copied()
                .collect(),
        )
    }

    fn bury_seat(&mut self, entity: Entity, scroll_due: bool) {
        let mut fallen_hero_team = None;
        for index in 0..self.seats.len() {
            if self.seats[index].unit == Some(entity) {
                let level = self.seats[index].level;
                self.let_souls_go(entity);
                self.requiem_on_death(entity);
                let mut bag = self.inventory.remove(entity).unwrap_or_default();
                // What a body carried when it fell sells back at the full price
                // no more (Dota: the sellback timer ends when the carrier dies).
                bag.slots
                    .iter_mut()
                    .flatten()
                    .for_each(|stack| stack.touched = true);
                if scroll_due {
                    // A town portal slot full to the byte keeps what it has.
                    bag.receive(ItemStack::free_scroll(self.seats[index].slot, self.tick));
                }
                let kept = Kept {
                    book: self.abilities.remove(entity).unwrap_or_default(),
                    bag,
                    modifiers: self.kept_through_death(entity),
                    fell: self.tick,
                };
                self.seats[index].unit = None;
                self.seats[index].kept = Some(kept);
                self.seats[index].deaths += 1;
                self.seats[index].respawn_left = respawn_wait(level);
                fallen_hero_team = Some(self.seats[index].team);
            }
            if self.seats[index].courier == Some(entity) {
                let mut carried = self.inventory.remove(entity);
                for stack in carried
                    .iter_mut()
                    .flat_map(|bag| bag.slots.iter_mut().flatten())
                {
                    stack.touched = true;
                }
                self.seats[index].courier_kept = carried;
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

/// Who learns of one fall.
#[derive(Clone, Copy)]
struct Told {
    /// The sides told it fell.
    fall: EventVisibility,
    /// The sides told who brought it down.
    killer: EventVisibility,
}

/// The side that is not this one.
pub(super) fn other_side(team: Team) -> Team {
    match team {
        Team::Radiant => Team::Dire,
        Team::Dire => Team::Radiant,
        Team::Neutral => Team::Neutral,
    }
}
