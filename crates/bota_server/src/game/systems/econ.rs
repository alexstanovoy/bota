//! What a kill pays, what the clock pays, and getting a hero back on its feet.

use bota_core::{GoldRange, denied_xp, hero_bounty, hero_kill_xp, hero_spawn_pos, kind_def, rules};
use bota_proto::{EventKind, Team, UnitKind, Vec2};

use crate::game::{Bounty, Entity, Event, EventVisibility, Landed, Level, Purpose, World, wire_id};

use super::fight::other_side;

impl World {
    /// Hands out the gold that arrives on its own: one every
    /// [`rules::PASSIVE_GOLD_PERIOD_TICKS`] after the horn, to every seat.
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
    /// Gold goes to the seat whose hero struck last, drawn evenly from the
    /// fallen's [`GoldRange`] at that moment; experience is shared
    /// among the killer side's living heroes within [`rules::XP_RADIUS`], and
    /// the killer of a hero shares wherever it stands. Bringing down one of
    /// your own is a deny: it pays no gold, and only a denied lane creep
    /// gives the other side [`rules::DENIED_XP_PCT`] of its experience.
    /// A flagbearer not denied also pays its bonus, see
    /// [`World::pledge_flag_bonus`], and a tower its team bounty, see
    /// [`World::pay_team_bounty`].
    ///
    /// A hero's head is priced by its streak, which ends with it, and its
    /// death costs it its net worth over [`rules::DEATH_GOLD_LOSS_SHARE`],
    /// whoever struck the blow, and never more than it holds.
    ///
    /// The killing unit's gold income modifier scales the bounty once, after
    /// the bounty is composed and before it is credited.
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
                    gold: GoldRange::exactly(hero_bounty(i32::from(seat.streak))),
                    bonus: 0,
                    team: 0,
                    xp: hero_kill_xp(seat.xp, i32::from(seat.streak), i32::from(seat.level)),
                }
            }
            None => match self.bounty.get(fallen).copied() {
                Some(bounty) => bounty,
                None => return 0,
            },
        };
        if let Some(index) = fallen_seat {
            let seat = &mut self.seats[index];
            let loss = (seat.net_worth / rules::DEATH_GOLD_LOSS_SHARE).clamp(0, seat.gold);
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
                let gold = self.roll_gold(bounty.gold);
                paid = self.bounty_after(killer, gold);
                self.seats[index].gold += paid;
                self.seats[index].net_worth += paid;
            }
        }
        let striker_seat = killer.and_then(|k| self.seat_of(k));
        if !denied && bounty.bonus > 0 {
            paid += self.pay_flag_bonus(at, other_side(side), bounty.bonus, striker_seat);
        }
        if bounty.team > 0 {
            paid += self.pay_team_bounty(side, denied, bounty.team, striker_seat);
        }
        if denied {
            if self
                .kind
                .get(fallen)
                .copied()
                .is_some_and(|kind| kind_def(kind).lane_creep)
            {
                let xp = denied_xp(bounty.xp);
                self.grant_xp_around(at, other_side(side), xp, None, events);
            }
            return paid;
        }
        let Some(earners) = killer_side else {
            return paid;
        };
        let killer_seat = fallen_seat.and_then(|_| killer.and_then(|entity| self.seat_of(entity)));
        self.grant_xp_around(at, earners, bounty.xp, killer_seat, events);
        paid
    }

    /// Pays a fallen building's team bounty: `team` to every seat of the side
    /// that brought it down, living or not, or on a deny half of it to every
    /// seat of both sides, each through its hero's gold income. Answers what
    /// the striker's seat was paid.
    fn pay_team_bounty(
        &mut self,
        side: Team,
        denied: bool,
        team: i32,
        striker_seat: Option<usize>,
    ) -> i32 {
        let share = if denied { team / 2 } else { team };
        let mut to_striker = 0;
        for index in 0..self.seats.len() {
            if !denied && self.seats[index].team != other_side(side) {
                continue;
            }
            let paid = self.bounty_after(self.seats[index].unit, share);
            self.seats[index].gold += paid;
            self.seats[index].net_worth += paid;
            if striker_seat == Some(index) {
                to_striker = paid;
            }
        }
        to_striker
    }

    /// Sets a flagbearer's bonus: [`rules::FLAGBEARER_BONUS_GOLD`] and a
    /// share of one draw of its own gold range, drawn as it spawns.
    pub(super) fn pledge_flag_bonus(&mut self, flagbearer: Entity) {
        let Some(range) = self.bounty.get(flagbearer).map(|bounty| bounty.gold) else {
            return;
        };
        let drawn = self.roll_gold(range);
        if let Some(bounty) = self.bounty.get_mut(flagbearer) {
            bounty.bonus =
                rules::FLAGBEARER_BONUS_GOLD + drawn * rules::FLAGBEARER_BONUS_BOUNTY_PCT / 100;
        }
    }

    /// Pays a fallen flagbearer's bonus to every seat of the earning side
    /// whose living hero stands within [`rules::FLAGBEARER_BONUS_RADIUS`],
    /// and to the killer's seat wherever it stands, each scaled by its own
    /// hero's gold income. Answers what the killer's seat was paid.
    fn pay_flag_bonus(
        &mut self,
        at: Vec2,
        earners: Team,
        bonus: i32,
        killer_seat: Option<usize>,
    ) -> i32 {
        let radius = rules::units(rules::FLAGBEARER_BONUS_RADIUS);
        let mut to_killer = 0;
        for index in 0..self.seats.len() {
            let seat = &self.seats[index];
            if seat.team != earners {
                continue;
            }
            let Some(unit) = seat.unit else {
                continue;
            };
            let near = self.alive(unit)
                && self
                    .transform
                    .get(unit)
                    .is_some_and(|t| t.pos.within(at, radius));
            if !near && killer_seat != Some(index) {
                continue;
            }
            let paid = self.bounty_after(Some(unit), bonus);
            self.seats[index].gold += paid;
            self.seats[index].net_worth += paid;
            if killer_seat == Some(index) {
                to_killer = paid;
            }
        }
        to_killer
    }

    /// One payout of a gold range, drawn evenly from it on the match's
    /// bounty stream. A range of one amount draws nothing.
    fn roll_gold(&mut self, range: GoldRange) -> i32 {
        if range.min == range.max {
            return range.min;
        }
        let span = u32::try_from(range.max - range.min + 1).expect("a bounty range is ordered");
        let rolled = range.min + self.rng.global(Purpose::Bounty).below(span) as i32;
        debug_assert!((range.min..=range.max).contains(&rolled));
        rolled
    }

    /// Adds blows a seat's hero landed on enemy heroes and structures to
    /// that seat's damage totals.
    pub fn credit_damage(&mut self, felt: &[Landed]) {
        for blow in felt {
            let Some(index) = blow.source.and_then(|source| self.seat_of(source)) else {
                continue;
            };
            if self.seats[index].team == blow.side {
                continue;
            }
            let Some(kind) = self.kind.get(blow.target).copied() else {
                continue;
            };
            let seat = &mut self.seats[index];
            if kind == UnitKind::Hero {
                seat.hero_damage = seat.hero_damage.saturating_add(blow.amount);
            } else if kind_def(kind).structure {
                seat.structure_damage = seat.structure_damage.saturating_add(blow.amount);
            }
        }
    }

    /// Which seat drives an entity, if any does.
    pub(super) fn seat_of(&self, entity: Entity) -> Option<usize> {
        self.seats.iter().position(|s| s.unit == Some(entity))
    }

    /// Shares experience among a side's heroes standing near a spot.
    fn grant_xp_around(
        &mut self,
        at: Vec2,
        team: Team,
        amount: i32,
        always: Option<usize>,
        events: &mut Vec<Event>,
    ) {
        if amount <= 0 {
            return;
        }
        let radius = rules::units(rules::XP_RADIUS);
        let earners: Vec<usize> = (0..self.seats.len())
            .filter(|&index| {
                let seat = &self.seats[index];
                seat.team == team
                    && seat.unit.is_some_and(|unit| {
                        self.alive(unit)
                            && (always == Some(index)
                                || self
                                    .transform
                                    .get(unit)
                                    .is_some_and(|t| t.pos.within(at, radius)))
                    })
            })
            .collect();
        if earners.is_empty() {
            return;
        }
        let share = amount / earners.len() as i32;
        for index in earners {
            self.grant_xp(index, share, events);
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
                    gold_hidden_from: None,
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

    /// Whether an entity is a hero.
    pub fn is_hero(&self, entity: Entity) -> bool {
        self.kind.get(entity) == Some(&UnitKind::Hero)
    }
}
