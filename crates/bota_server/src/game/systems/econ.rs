//! What a kill pays, what the clock pays, and getting a hero back on its feet.

use bota_core::{
    GoldRange, ModifierKind, assist_gold, denied_xp, hero_bounty, hero_kill_xp, hero_spawn_pos,
    kind_def, passive_gold_by, rules,
};
use bota_proto::{EventKind, Team, UnitKind, Vec2};

use crate::game::{
    Bounty, Entity, Event, EventVisibility, Landed, Level, Modifier, Purpose, World, wire_id,
};

use super::fight::other_side;

/// Net worths around a hero's fall, before it moved any gold.
#[derive(Clone, Copy, Debug)]
struct Worths {
    /// The fallen hero's own.
    fallen: i32,
    /// Its side's, its own included.
    fallen_side: i64,
    /// The side that brought it down.
    killer_side: i64,
}

impl World {
    /// Hands out the gold that arrives on its own, to every seat: what
    /// [`passive_gold_by`] owes for this tick of the clock.
    pub fn passive_gold(&mut self) {
        if self.clock() <= rules::PREGAME_TICKS {
            return;
        }
        let since = self.clock() - rules::PREGAME_TICKS;
        let due = passive_gold_by(since) - passive_gold_by(since - 1);
        if due == 0 {
            return;
        }
        for seat in self.seats.iter_mut() {
            seat.gold += due;
            seat.net_worth += due;
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
    /// A hero's head is priced by its level and its streak, which ends with
    /// it, and First Blood; a hero brought down by an enemy hero also pays
    /// assist gold, see [`World::pay_assist_gold`]. Its death costs it its
    /// net worth over [`rules::DEATH_GOLD_LOSS_SHARE`], whoever struck the
    /// blow, and never more than it holds.
    ///
    /// The killing unit's gold income modifier scales the bounty once, after
    /// the bounty is composed and before it is credited.
    pub fn pay_for(
        &mut self,
        fallen: Entity,
        killer: Option<Entity>,
        events: &mut Vec<Event>,
    ) -> i32 {
        let Some((bounty, fallen_seat, worths)) = self.price_of_fall(fallen) else {
            return 0;
        };
        let Some(side) = self.team.get(fallen).copied() else {
            return 0;
        };
        let at = self.pos(fallen).unwrap_or(Vec2::ZERO);
        let killer_side = killer.and_then(|k| self.team.get(k).copied());
        let denied = killer_side == Some(side);
        let mut paid = self.credit_killer(killer, denied, fallen_seat.is_some(), bounty.gold);
        let striker_seat = killer.and_then(|k| self.seat_of(k));
        if let Some(worths) = worths
            && !denied
            && striker_seat.is_some()
        {
            paid += self.pay_assist_gold(at, other_side(side), worths, striker_seat);
        }
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

    /// What the fallen is worth, the seat it was a hero of and the net
    /// worths around a hero's fall as they stood before the death moved any
    /// gold: a hero's head is priced by its level and streak and by whether
    /// it is the match's first, and its death costs its seat the share of
    /// its net worth. Nothing for a fallen that is worth nothing.
    fn price_of_fall(&mut self, fallen: Entity) -> Option<(Bounty, Option<usize>, Option<Worths>)> {
        let fallen_seat = self.seat_of(fallen);
        let Some(index) = fallen_seat else {
            let bounty = self.bounty.get(fallen).copied()?;
            return Some((bounty, None, None));
        };
        let first_blood = self.seats.iter().all(|seat| seat.kills == 0);
        let seat = &self.seats[index];
        let (level, streak) = (i32::from(seat.level), i32::from(seat.streak));
        let bounty = Bounty {
            gold: GoldRange::exactly(hero_bounty(level, streak, first_blood)),
            bonus: 0,
            team: 0,
            xp: hero_kill_xp(seat.xp, streak, level),
        };
        let worths = Worths {
            fallen: seat.net_worth,
            fallen_side: self.side_worth(seat.team),
            killer_side: self.side_worth(other_side(seat.team)),
        };
        let seat = &mut self.seats[index];
        let loss = (seat.net_worth / rules::DEATH_GOLD_LOSS_SHARE).clamp(0, seat.gold);
        seat.gold -= loss;
        seat.net_worth -= loss;
        seat.streak = 0;
        Some((bounty, fallen_seat, Some(worths)))
    }

    fn side_worth(&self, side: Team) -> i64 {
        self.seats
            .iter()
            .filter(|seat| seat.team == side)
            .map(|seat| i64::from(seat.net_worth))
            .sum()
    }

    /// Pays the assist gold of a hero an enemy hero brought down to every
    /// seat of the earning side whose living hero stands within
    /// [`rules::XP_RADIUS`] of the fall, and to the killer's seat wherever
    /// it stands, each through its own hero's gold income: the same
    /// [`assist_gold`] to each. Answers what the killer's seat was paid.
    fn pay_assist_gold(
        &mut self,
        at: Vec2,
        earners: Team,
        worths: Worths,
        killer_seat: Option<usize>,
    ) -> i32 {
        let paid_seats = self.seats_near(at, earners, rules::XP_RADIUS, killer_seat);
        let share = assist_gold(
            worths.fallen,
            paid_seats.len(),
            worths.killer_side,
            worths.fallen_side,
        );
        self.pay_seats(paid_seats, share, killer_seat)
    }

    /// Every seat of a side whose living hero stands within `radius` of a
    /// spot, and the killer's seat wherever it stands.
    fn seats_near(
        &self,
        at: Vec2,
        side: Team,
        radius: i32,
        killer_seat: Option<usize>,
    ) -> Vec<usize> {
        let radius = rules::units(radius);
        (0..self.seats.len())
            .filter(|&index| {
                let seat = &self.seats[index];
                seat.team == side
                    && (killer_seat == Some(index)
                        || seat.unit.is_some_and(|unit| {
                            self.alive(unit)
                                && self.pos(unit).is_some_and(|pos| pos.within(at, radius))
                        }))
            })
            .collect()
    }

    /// Pays each seat `amount` through its hero's gold income, in order.
    /// Answers what the killer's seat was paid.
    fn pay_seats(
        &mut self,
        seats: impl IntoIterator<Item = usize>,
        amount: i32,
        killer_seat: Option<usize>,
    ) -> i32 {
        let mut to_killer = 0;
        for index in seats {
            let paid = self.bounty_after(self.seats[index].unit, amount);
            self.seats[index].gold += paid;
            self.seats[index].net_worth += paid;
            if killer_seat == Some(index) {
                to_killer = paid;
            }
        }
        to_killer
    }

    /// Credits the seat of the killer: a deny, or a streak step for a hero and
    /// a last hit for anything else, with the gold drawn from `gold`. Answers
    /// the gold paid.
    fn credit_killer(
        &mut self,
        killer: Option<Entity>,
        denied: bool,
        fallen_was_hero: bool,
        gold: GoldRange,
    ) -> i32 {
        let Some(index) = killer.and_then(|k| self.seat_of(k)) else {
            return 0;
        };
        if denied {
            self.seats[index].denies += 1;
            return 0;
        }
        if fallen_was_hero {
            self.seats[index].streak += 1;
        } else {
            self.seats[index].last_hits += 1;
        }
        let gold = self.roll_gold(gold);
        self.pay_seats([index], gold, Some(index))
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
        let paid: Vec<usize> = (0..self.seats.len())
            .filter(|&index| denied || self.seats[index].team == other_side(side))
            .collect();
        self.pay_seats(paid, share, striker_seat)
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
        let paid_seats = self.seats_near(at, earners, rules::FLAGBEARER_BONUS_RADIUS, killer_seat);
        self.pay_seats(paid_seats, bonus, killer_seat)
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
        let mut earners = self.seats_near(at, team, rules::XP_RADIUS, always);
        earners.retain(|&index| self.seats[index].unit.is_some_and(|unit| self.alive(unit)));
        if earners.is_empty() {
            return;
        }
        let share = amount / earners.len() as i32;
        for index in earners {
            self.grant_xp(index, share, events);
        }
    }

    /// Adds experience to a seat, taking its hero up the levels it passes; a
    /// level's point goes into the Attribute Bonus by itself when nothing
    /// else could take it ([`crate::game::AbilityBook::auto_attribute_bonus`]).
    pub fn grant_xp(&mut self, seat: usize, amount: i32, events: &mut Vec<Event>) {
        self.seats[seat].xp += amount;
        while self.seats[seat].level < rules::HERO_MAX_LEVEL {
            let next = rules::XP_THRESHOLDS[usize::from(self.seats[seat].level)];
            if self.seats[seat].xp < next {
                break;
            }
            self.seats[seat].level += 1;
            let level = self.seats[seat].level;
            let book = match self.seats[seat].unit {
                Some(unit) => self.abilities.get_mut(unit),
                None => self.seats[seat].kept.as_mut().map(|kept| &mut kept.book),
            };
            if let Some(book) = book {
                book.auto_attribute_bonus(level);
            }
            self.raise_courier(seat);
            if let Some(unit) = self.seats[seat].unit {
                self.level.insert(unit, Level(level));
                events.push(Event {
                    kind: EventKind::LevelUp {
                        unit: wire_id(unit),
                        level,
                    },
                    visible_to: self.who_may_know(unit, None),
                    gold_hidden_from: None,
                    source_told_to: EventVisibility::Everyone,
                });
            }
        }
    }

    /// Runs down every respawn timer and puts the heroes back at their
    /// fountains.
    pub fn tick_respawns(&mut self) {
        for index in 0..self.seats.len() {
            if self.seats[index].unit.is_some() {
                continue;
            }
            self.seats[index].respawn_left = self.seats[index].respawn_left.saturating_sub(1);
            if self.seats[index].respawn_left > 0 {
                continue;
            }
            self.respawn(index);
        }
    }

    /// Puts a seat's fallen hero back at its fountain now, with what it
    /// left behind.
    pub(crate) fn respawn(&mut self, index: usize) {
        let map = self.map;
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
            for held in kept.modifiers.0 {
                self.put_modifier(unit, held);
            }
        }
        // Risen at its fountain, untouchable until its player gives it a
        // command or the fountain's Rejuvenation no longer holds on it
        // (Dota's modifier_fountain_invulnerability).
        self.put_modifier(
            unit,
            Modifier::new(ModifierKind::FountainInvulnerable, Some(unit), None, None),
        );
        self.seats[index].respawn_left = 0;
        self.seats[index].unit = Some(unit);
        self.settle();
    }

    pub fn is_hero(&self, entity: Entity) -> bool {
        self.kind.get(entity) == Some(&UnitKind::Hero)
    }

    /// Whether an entity is a hero and no illusion of one: what wears
    /// Dota's `modifier_illusion` is a hero to look at alone.
    pub fn is_real_hero(&self, entity: Entity) -> bool {
        self.is_hero(entity) && !self.wears_illusion(entity)
    }

    /// Takes a risen hero's fountain invulnerability off once the
    /// Rejuvenation Aura that provides it no longer holds on it: it walked
    /// out of the fountain's reach and the aura's linger ran out.
    pub fn lift_fountain_invulnerability(&mut self) {
        for index in 0..self.seats.len() {
            let Some(hero) = self.seats[index].unit else {
                continue;
            };
            let Some(on_it) = self.modifiers.get(hero) else {
                continue;
            };
            let risen = on_it
                .active()
                .any(|held| held.kind == ModifierKind::FountainInvulnerable);
            let mended = on_it
                .active()
                .any(|held| matches!(held.kind, ModifierKind::Fountain { .. }));
            if risen && !mended {
                self.take_modifier(hero, ModifierKind::FountainInvulnerable, Some(hero));
            }
        }
    }

    /// Whether an entity wears what makes a unit an illusion.
    pub fn wears_illusion(&self, entity: Entity) -> bool {
        self.modifiers.get(entity).is_some_and(wears_illusion)
    }
}

/// Whether what is on a unit makes it an illusion.
pub(crate) fn wears_illusion(on_it: &crate::game::Modifiers) -> bool {
    on_it
        .active()
        .any(|held| held.kind == bota_core::ModifierKind::Illusion)
}
