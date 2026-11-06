//! What a kill pays, what the clock pays, and getting a hero back on its feet.

use bota_core::{
    GoldRange, ModifierKind, assist_gold, denied_xp, hero_bounty, hero_kill_xp, hero_spawn_pos,
    kind_def, passive_gold_by, rules,
};
use bota_proto::{EventKind, GoldAward, GoldAwardKind, Team, UnitKind, Vec2};

use crate::game::{
    Bounty, Entity, Event, EventVisibility, KILL_CREDIT_WINDOW_TICKS, Landed, Level, Modifier,
    Purpose, World, wire_id,
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

/// Who receives a hero's bounty and whether this death ends a killing streak.
struct HeroCredit {
    credited: Option<usize>,
    earners: Vec<usize>,
    counts_as_kill: bool,
}

impl World {
    /// Hands out the gold that arrives on its own, to every seat: what
    /// [`passive_gold_by`] owes for this tick of the clock.
    pub fn passive_gold(&mut self) {
        let horn = self.schedule().horn;
        if self.clock() <= horn {
            return;
        }
        let since = self.clock() - horn;
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
    /// Gold goes to the credited killer: the last hero to strike, or the sole
    /// recent hero damager when a hostile creep or tower finishes it. It is
    /// drawn evenly from the
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
        self.pay_for_report(fallen, killer, events).0
    }

    /// The same credits, with the factual components to attach to the death event.
    pub(crate) fn pay_for_report(
        &mut self,
        fallen: Entity,
        killer: Option<Entity>,
        events: &mut Vec<Event>,
    ) -> (i32, Option<Vec<GoldAward>>) {
        let credit = self.hero_credit(fallen, killer);
        let reward_source = credit
            .as_ref()
            .and_then(|credit| credit.credited)
            .and_then(|index| self.seats[index].unit)
            .or(killer);
        let counts_as_kill = credit.as_ref().is_some_and(|credit| credit.counts_as_kill);
        let Some((mut bounty, fallen_seat, worths)) = self.price_of_fall(fallen, counts_as_kill)
        else {
            return (0, None);
        };
        if self.wears_illusion(fallen) && !killer.is_some_and(|killer| self.is_hero(killer)) {
            bounty.xp = 0;
        }
        let Some(side) = self.team.get(fallen).copied() else {
            return (0, None);
        };
        let at = self.pos(fallen).unwrap_or(Vec2::ZERO);
        let killer_side = killer.and_then(|unit| self.team.get(unit).copied());
        let denied = killer_side == Some(side);
        let striker_seat = reward_source.and_then(|unit| self.seat_of(unit));
        let (mut paid, awards) =
            self.pay_death_components(fallen, reward_source, bounty, credit.as_ref(), worths);
        if bounty.team > 0 {
            // The side that brought it down: a lane building's enemy, and
            // for a neutral the side of whoever struck the blow.
            let earners = if side == Team::Neutral {
                killer_side
            } else {
                Some(other_side(side))
            };
            paid += self.pay_team_bounty(earners, denied, bounty.team, striker_seat);
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
            return (paid, awards);
        }
        let Some(earners) = killer_side else {
            return (paid, awards);
        };
        let killer_seat =
            fallen_seat.and_then(|_| reward_source.and_then(|entity| self.seat_of(entity)));
        self.grant_xp_around(at, earners, bounty.xp, killer_seat, events);
        (paid, awards)
    }

    /// Credits the bounty and each additional gold component before the
    /// contribution ledger is spent by the scoreboard's assist stage.
    fn pay_death_components(
        &mut self,
        fallen: Entity,
        killer: Option<Entity>,
        bounty: Bounty,
        credit: Option<&HeroCredit>,
        worths: Option<Worths>,
    ) -> (i32, Option<Vec<GoldAward>>) {
        let side = *self.team.get(fallen).expect("a priced fall has a side");
        let at = self.pos(fallen).unwrap_or(Vec2::ZERO);
        let denied = killer.and_then(|unit| self.team.get(unit).copied()) == Some(side);
        let striker_seat = killer.and_then(|unit| self.seat_of(unit));
        let detailed = !denied && (credit.is_some() || bounty.bonus > 0);
        let before = detailed.then(|| self.gold_balances());
        let mut paid = self.credit_killer(killer, denied, worths.is_some(), bounty.gold);
        if striker_seat.is_none()
            && let Some(credit) = credit
            && !credit.earners.is_empty()
        {
            let share = self.roll_gold(bounty.gold)
                / i32::try_from(credit.earners.len()).expect("seats are few");
            self.pay_seats(credit.earners.iter().copied(), share, None);
        }
        let mut awards =
            before.map(|before| self.gold_awards_since(&before, GoldAwardKind::KillBounty));
        if let Some(worths) = worths
            && credit.is_some()
        {
            let before = self.gold_balances();
            paid += self.pay_assist_gold(fallen, at, other_side(side), worths, striker_seat);
            Self::append_gold_awards(
                &mut awards,
                self.gold_awards_since(&before, GoldAwardKind::AssistGold),
            );
        }
        if !denied && bounty.bonus > 0 {
            let (bonus, breakdown) =
                self.report_flag_bonus(at, other_side(side), bounty.bonus, striker_seat);
            paid += bonus;
            Self::append_gold_awards(&mut awards, breakdown);
        }
        if worths.is_some()
            && striker_seat.is_none()
            && let Some(awards) = &awards
        {
            // There is no personal aggregate for a shared kill: the feed tells
            // the total actually paid, while awards retain every recipient.
            paid = awards
                .iter()
                .map(|award| i64::from(award.amount))
                .sum::<i64>()
                .min(i64::from(i32::MAX)) as i32;
        }
        (paid, awards)
    }

    /// What the fallen is worth, the seat it was a hero of and the net
    /// worths before gold moves. Only a credited hero kill or shared assisted
    /// kill claims First Blood and ends a streak; an undamaged creep/tower
    /// finish shares the base bounty without claiming those rewards [LIQ Gold].
    fn price_of_fall(
        &mut self,
        fallen: Entity,
        counts_as_kill: bool,
    ) -> Option<(Bounty, Option<usize>, Option<Worths>)> {
        let fallen_seat = self.seat_of(fallen);
        let Some(index) = fallen_seat else {
            let bounty = self.bounty.get(fallen).copied()?;
            return Some((bounty, None, None));
        };
        let first_blood = counts_as_kill
            && !self.first_blood_claimed
            && self.seats.iter().all(|seat| seat.kills == 0);
        self.first_blood_claimed |= counts_as_kill;
        let seat = &self.seats[index];
        let (level, streak) = (i32::from(seat.level), i32::from(seat.streak));
        let bounty = Bounty {
            gold: GoldRange::exactly(hero_bounty(
                level,
                if counts_as_kill { streak } else { 0 },
                first_blood,
            )),
            bonus: 0,
            team: 0,
            xp: hero_kill_xp(seat.xp, if counts_as_kill { streak } else { 0 }, level),
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
        if counts_as_kill {
            seat.streak = 0;
        }
        Some((bounty, fallen_seat, Some(worths)))
    }

    fn side_worth(&self, side: Team) -> i64 {
        self.seats
            .iter()
            .filter(|seat| seat.team == side)
            .map(|seat| i64::from(seat.net_worth))
            .sum()
    }

    /// Pays assist gold to living nearby heroes, the killer, and recent damage
    /// or debuff contributors even if distant or dead [LIQ Gold]. Each receives
    /// the same share through its own gold income modifier.
    /// The wiki does not specify an AoE-gold contribution window; we use the
    /// same twenty seconds as scoreboard assists as an explicit simulation policy.
    fn pay_assist_gold(
        &mut self,
        fallen: Entity,
        at: Vec2,
        earners: Team,
        worths: Worths,
        killer_seat: Option<usize>,
    ) -> i32 {
        let mut paid_seats = self.seats_near(at, earners, rules::ASSIST_GOLD_RADIUS, killer_seat);
        if let Some(index) = self.seat_of(fallen) {
            paid_seats.extend(self.seats[index].assisted_by(self.tick, None));
        }
        paid_seats.retain(|&index| self.seats[index].team == earners);
        paid_seats.sort_unstable();
        paid_seats.dedup();
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
    pub(super) fn pay_seats(
        &mut self,
        seats: impl IntoIterator<Item = usize>,
        amount: i32,
        killer_seat: Option<usize>,
    ) -> i32 {
        let mut to_killer = 0;
        for index in seats {
            let paid = self.bounty_after(self.seats[index].unit, amount);
            let seat = &mut self.seats[index];
            let capacity = i32::MAX
                .saturating_sub(seat.gold)
                .min(i32::MAX.saturating_sub(seat.net_worth))
                .max(0);
            let paid = paid.min(capacity);
            seat.gold += paid;
            seat.net_worth += paid;
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

    /// Pays a fallen unit's team bounty: `team` to every seat of `earners`,
    /// the side that brought it down, living or not, or on a deny half of it
    /// to every seat of both sides, each through its hero's gold income.
    /// Answers what the striker's seat was paid.
    fn pay_team_bounty(
        &mut self,
        earners: Option<Team>,
        denied: bool,
        team: i32,
        striker_seat: Option<usize>,
    ) -> i32 {
        let share = if denied { team / 2 } else { team };
        let paid: Vec<usize> = (0..self.seats.len())
            .filter(|&index| denied || Some(self.seats[index].team) == earners)
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

    /// Reports this payout stage's actual flagbearer credits without replacing
    /// components already reported by another stage.
    fn report_flag_bonus(
        &mut self,
        at: Vec2,
        earners: Team,
        bonus: i32,
        killer_seat: Option<usize>,
    ) -> (i32, Vec<GoldAward>) {
        let before = self.gold_balances();
        let paid = self.pay_flag_bonus(at, earners, bonus, killer_seat);
        (
            paid,
            self.gold_awards_since(&before, GoldAwardKind::FlagbearerBonus),
        )
    }

    fn gold_balances(&self) -> Vec<i32> {
        self.seats.iter().map(|seat| seat.gold).collect()
    }

    fn append_gold_awards(awards: &mut Option<Vec<GoldAward>>, more: Vec<GoldAward>) {
        let awards = awards.get_or_insert_with(Vec::new);
        assert!(awards.len() + more.len() <= bota_proto::MAX_DEATH_GOLD_AWARDS);
        awards.extend(more);
    }

    /// Reports each stage's actual income-scaled credits, once per recipient.
    fn gold_awards_since(&self, before: &[i32], kind: GoldAwardKind) -> Vec<GoldAward> {
        assert_eq!(before.len(), self.seats.len());
        self.seats
            .iter()
            .zip(before)
            .filter_map(|(seat, before)| {
                let amount = seat.gold - before;
                (amount > 0).then_some(GoldAward {
                    recipient: seat.slot,
                    team: seat.team,
                    kind,
                    amount,
                })
            })
            .collect()
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

    /// Notes, for the assists of a fall, the blows an enemy hero landed on a hero (a blow that did
    /// no damage counts: Dota credits a negated one as well).
    pub fn note_hurt(&mut self, felt: &[Landed]) {
        for blow in felt {
            self.note_hero_contribution(blow.source, blow.target, blow.amount > 0);
        }
    }

    /// Notes that `source`, if it is a seat's hero, hurt `target`, if that is an enemy seat's.
    pub(super) fn note_hurt_by(&mut self, source: Option<Entity>, target: Entity) {
        self.note_hero_contribution(source, target, false);
    }

    fn note_hero_contribution(&mut self, source: Option<Entity>, target: Entity, damage: bool) {
        let (Some(by), Some(hurt)) = (source.and_then(|s| self.seat_of(s)), self.seat_of(target))
        else {
            return;
        };
        if self.seats[by].team != self.seats[hurt].team {
            let tick = self.tick;
            if damage {
                self.seats[hurt].note_damage_by(by, tick);
            } else {
                self.seats[hurt].note_hurt_by(by, tick);
            }
        }
    }

    /// A hostile creep or tower finish credits the sole recent damager.
    pub(super) fn credited_hero(&self, fallen: Entity, killer: Option<Entity>) -> Option<Entity> {
        if !self.is_real_hero(fallen) {
            return None;
        }
        self.hero_credit(fallen, killer)
            .and_then(|credit| credit.credited)
            .and_then(|seat| self.seats[seat].unit)
    }

    /// Hero bounty recipients for the three nonhero-finisher cases [LIQ Gold]:
    /// one damager owns the kill; several share with helpers; none share across
    /// the enemy team without claiming First Blood or ending a streak.
    fn hero_credit(&self, fallen: Entity, killer: Option<Entity>) -> Option<HeroCredit> {
        let victim = self.seat_of(fallen)?;
        let source = killer?;
        let side = *self.team.get(source)?;
        if side == Team::Neutral || side == self.seats[victim].team {
            return None;
        }
        if let Some(index) = self.seat_of(source) {
            return Some(HeroCredit {
                credited: Some(index),
                earners: vec![index],
                counts_as_kill: true,
            });
        }
        let kind = *self.kind.get(source)?;
        if !kind_def(kind).lane_creep && kind != UnitKind::Tower {
            return None;
        }
        let mut damagers = self.seats[victim].hurt_by.iter().filter(|held| {
            held.damage_at
                .is_some_and(|tick| self.tick.saturating_sub(tick) <= KILL_CREDIT_WINDOW_TICKS)
        });
        let (credited, mut earners, counts_as_kill) = match (damagers.next(), damagers.next()) {
            (Some(held), None) => (Some(held.by), vec![held.by], true),
            (Some(_), Some(_)) => (None, self.seats[victim].assisted_by(self.tick, None), true),
            _ => (
                None,
                (0..self.seats.len())
                    .filter(|&index| self.seats[index].team == side)
                    .collect(),
                false,
            ),
        };
        earners.retain(|&index| self.seats[index].team == side);
        Some(HeroCredit {
            credited,
            earners,
            counts_as_kill,
        })
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
            let book = self.seats[seat]
                .unit
                .and_then(|unit| self.abilities.get_mut(unit));
            if let Some(book) = book {
                book.auto_attribute_bonus(level);
            }
            self.level_courier(seat);
            if let Some(unit) = self.seats[seat].unit {
                self.level.insert(unit, Level(level));
                events.push(Event::Fact {
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
            if !self.seats[index].dead() {
                continue;
            }
            self.seats[index].respawn_left -= 1;
            if !self.seats[index].dead() {
                self.respawn(index);
            }
        }
    }

    /// Stands a seat's fallen hero up at its fountain now.
    pub(crate) fn respawn(&mut self, index: usize) {
        let at = hero_spawn_pos(self.map, self.seats[index].team);
        self.raise_fallen_hero(index, at);
        let Some(hero) = self.seats[index].unit else {
            return;
        };
        // Risen at its fountain, untouchable until its player gives it a
        // command or the fountain's Rejuvenation no longer holds on it
        // (Dota's modifier_fountain_invulnerability).
        self.put_modifier(
            hero,
            Modifier::new(ModifierKind::FountainInvulnerable, Some(hero), None, None),
        );
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
