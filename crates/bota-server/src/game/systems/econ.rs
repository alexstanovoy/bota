//! What a kill pays, what the clock pays, and getting a hero back on its feet.

use bota_proto::{EventKind, Team, UnitKind, Vec2};

use crate::game::{Bounty, Entity, Level, World, wire_id};
use crate::game::{Event, EventVisibility, hero_spawn_pos, rules};

impl World {
    /// Hands out the gold that arrives on its own: one a period, to every
    /// seat.
    pub fn passive_gold(&mut self) {
        if self.tick <= rules::PREGAME_TICKS
            || !(self.tick - rules::PREGAME_TICKS).is_multiple_of(rules::PASSIVE_GOLD_PERIOD_TICKS)
        {
            return;
        }
        for seat in self.seats.iter_mut() {
            seat.gold += 1;
            seat.net_worth += 1;
        }
    }

    /// Pays for one entity brought down, and says how much gold that paid.
    ///
    /// Gold goes to whoever struck last; experience is shared among the
    /// enemy heroes standing near enough to see it fall. Bringing down one of
    /// your own is a deny: it pays the other side nothing.
    ///
    /// A hero's head is priced by its streak, which ends with it, and its
    /// death costs it gold by its level — whoever struck the blow, and never
    /// more than it holds.
    pub fn pay_for(
        &mut self,
        fallen: Entity,
        killer: Option<Entity>,
        events: &mut Vec<Event>,
    ) -> i32 {
        let fallen_seat = self.seat_of(fallen);
        let bounty = match fallen_seat {
            Some(index) => {
                let seat = &self.seats[index];
                Bounty {
                    gold: World::hero_bounty(i32::from(seat.streak)),
                    xp: rules::HERO_KILL_XP_BASE
                        + rules::HERO_KILL_XP_PER_LEVEL * i32::from(seat.level),
                }
            }
            None => match self.bounty.get(fallen).copied() {
                Some(bounty) => bounty,
                None => return 0,
            },
        };
        if let Some(index) = fallen_seat {
            let seat = &mut self.seats[index];
            let loss = (rules::DEATH_GOLD_LOSS_PER_LEVEL * i32::from(seat.level)).min(seat.gold);
            seat.gold -= loss;
            seat.net_worth -= loss;
            seat.streak = 0;
        }
        let Some(side) = self.team.get(fallen).copied() else {
            return 0;
        };
        let at = self.transform.get(fallen).map_or(Vec2::ZERO, |t| t.pos);
        let killer_side = killer.and_then(|k| self.team.get(k).copied());
        let denied = killer_side == Some(side);
        let mut paid = 0;
        if let Some(index) = killer.and_then(|k| self.seat_of(k)) {
            if denied {
                self.seats[index].denies += 1;
            } else {
                if fallen_seat.is_some() {
                    self.seats[index].streak += 1;
                } else {
                    self.seats[index].last_hits += 1;
                }
                self.seats[index].gold += bounty.gold;
                self.seats[index].net_worth += bounty.gold;
                paid = bounty.gold;
            }
        }
        if denied {
            return 0;
        }
        let Some(earners) = killer_side else {
            return paid;
        };
        self.grant_xp_around(at, earners, bounty.xp, events);
        paid
    }

    /// Which seat drives an entity, if any does.
    fn seat_of(&self, entity: Entity) -> Option<usize> {
        self.seats.iter().position(|s| s.unit == Some(entity))
    }

    /// Shares experience among a side's heroes standing near a spot.
    fn grant_xp_around(&mut self, at: Vec2, team: Team, amount: i32, events: &mut Vec<Event>) {
        if amount <= 0 {
            return;
        }
        let radius = rules::units(rules::XP_RADIUS);
        let earners: Vec<usize> = (0..self.seats.len())
            .filter(|&index| {
                let seat = &self.seats[index];
                seat.team == team
                    && seat
                        .unit
                        .and_then(|unit| self.transform.get(unit))
                        .is_some_and(|t| t.pos.within(at, radius))
            })
            .collect();
        for index in earners {
            self.grant_xp(index, amount, events);
        }
    }

    /// Adds experience to a seat, taking its hero up the levels it passes.
    pub fn grant_xp(&mut self, seat: usize, amount: i32, events: &mut Vec<Event>) {
        self.seats[seat].xp += amount;
        while self.seats[seat].level < rules::HERO_MAX_LEVEL {
            let next = rules::XP_THRESHOLDS[usize::from(self.seats[seat].level)];
            if self.seats[seat].xp < next {
                break;
            }
            self.seats[seat].level += 1;
            let level = self.seats[seat].level;
            if let Some(unit) = self.seats[seat].unit {
                self.level.insert(unit, Level(level));
                events.push(Event {
                    kind: EventKind::LevelUp {
                        unit: wire_id(unit),
                        level,
                    },
                    visible_to: EventVisibility::Everyone,
                });
            }
        }
    }

    /// Runs down every respawn timer and puts the heroes back at their
    /// fountains.
    pub fn tick_respawns(&mut self) {
        let map = self.map;
        for index in 0..self.seats.len() {
            if self.seats[index].unit.is_some() {
                continue;
            }
            self.seats[index].respawn_left = self.seats[index].respawn_left.saturating_sub(1);
            if self.seats[index].respawn_left > 0 {
                continue;
            }
            let (team, slot, hero, level) = (
                self.seats[index].team,
                self.seats[index].slot,
                self.seats[index].hero,
                self.seats[index].level,
            );
            let at = hero_spawn_pos(map, team);
            let unit = self.spawn_hero(team, at, slot, hero);
            self.level.insert(unit, Level(level));
            if let Some(kept) = self.seats[index].kept.take() {
                self.abilities.insert(unit, kept.book);
                self.inventory.insert(unit, kept.bag);
                self.stacks.insert(unit, kept.stacks);
            }
            self.seats[index].unit = Some(unit);
            self.settle();
        }
    }

    /// How long a seat waits before its hero comes back.
    pub fn respawn_wait(level: u8) -> u32 {
        rules::RESPAWN_BASE_TICKS + rules::RESPAWN_PER_LEVEL_TICKS * u32::from(level)
    }

    /// What bringing down a hero on a streak of so many pays.
    pub fn hero_bounty(streak: i32) -> i32 {
        rules::HERO_KILL_BOUNTY_BASE
            + rules::HERO_KILL_BOUNTY_PER_STREAK * streak.min(rules::HERO_KILL_STREAK_CAP)
    }

    /// Whether an entity is a hero.
    pub fn is_hero(&self, entity: Entity) -> bool {
        self.kind.get(entity) == Some(&UnitKind::Hero)
    }
}
