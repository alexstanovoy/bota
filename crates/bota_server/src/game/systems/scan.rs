//! Scan: a side's shared charges, and the scans that look for enemy heroes.
//!
//! Dota's team ability. One ledger of charges serves a whole side. Using it sets a radar at the
//! point, which looks at once and then every second for eight seconds, and the side is told after
//! every look whether an enemy hero stood in the area, and nothing else.
//!
//! A radar is the side's own record, not an entity of the world: nothing meets it, and it takes
//! nothing from the sequences the world shares (handles, slots, draws), so that what the other
//! side is told is the same whether a scan happened or not.

use bota_core::{ScanLedger, rules, side_index};
use bota_proto::{EventKind, MAX_SCAN_RADARS, RadarView, RejectReason, SlotId, Team, Vec2};

use crate::game::{Event, EventVisibility, Radar, ScanSide, World};

impl World {
    /// What a side's Scan holds now: its ledger as the clock leaves it, or what the clock alone
    /// says while nothing has touched it.
    pub fn scan_ledger(&self, index: usize) -> ScanLedger {
        let clock = self.clock();
        match self.scans.0[index] {
            Some(mut held) => {
                held.ledger.catch_up(clock);
                held.ledger
            }
            None => ScanLedger::untouched(self.schedule(), clock),
        }
    }

    /// A side's Scan, taken out of the clock's hands: from here on the match carries it.
    fn scan_side_mut(&mut self, index: usize) -> &mut ScanSide {
        let ledger = self.scan_ledger(index);
        let held = self.scans.0[index].get_or_insert(ScanSide::new(ledger));
        held.ledger = ledger;
        held
    }

    /// Whether a seat's side may use its Scan now: it holds a charge, and no more than
    /// [`MAX_SCAN_RADARS`] of its scans are looking (a bound only a cheat's refresh can reach).
    /// Nothing else is asked, as in Dota: not the hero's life, not its mana, not where the point is.
    pub fn check_scan(&self, slot: SlotId) -> Result<(), RejectReason> {
        let seat = self.seat(slot).ok_or(RejectReason::HeroDead)?;
        let index = side_index(seat.team).ok_or(RejectReason::NotYourUnit)?;
        let looking = self.scans.0[index].map_or(0, |held| held.looking());
        if self.scan_ledger(index).charges == 0 || looking >= MAX_SCAN_RADARS {
            return Err(RejectReason::NoScanCharge);
        }
        Ok(())
    }

    /// Uses the Scan of a seat's side at a point, if it holds a charge: spends it and sets a
    /// radar there that looks once at once.
    pub(crate) fn use_scan(&mut self, slot: SlotId, at: Vec2) {
        if self.check_scan(slot).is_err() {
            return;
        }
        let Some(team) = self.seat(slot).map(|seat| seat.team) else {
            return;
        };
        let Some(index) = side_index(team) else {
            return;
        };
        let clock = self.clock();
        let found = self.enemy_hero_within(team, at, rules::units(rules::SCAN_RADIUS));
        let held = self.scan_side_mut(index);
        assert!(
            held.ledger.spend(clock),
            "a side that holds a charge spends it"
        );
        let place = held
            .radars
            .iter()
            .position(Option::is_none)
            .expect("check_scan leaves a place free");
        held.radars[place] = Some(Radar {
            at,
            ticks_left: rules::SCAN_DURATION_TICKS,
            next_look: rules::SCAN_LOOK_TICKS,
            found,
        });
    }

    /// Lets the charges arrive that are due, lets every radar that has run out go, and has the
    /// rest look again when a look is due.
    pub(crate) fn tick_scans(&mut self) {
        let clock = self.clock();
        for held in self.scans.0.iter_mut().flatten() {
            held.ledger.catch_up(clock);
        }
        for (index, team) in [Team::Radiant, Team::Dire].into_iter().enumerate() {
            let Some(mut held) = self.scans.0[index] else {
                continue;
            };
            for place in &mut held.radars {
                let Some(radar) = place else {
                    continue;
                };
                if radar.ticks_left == 0 {
                    *place = None;
                    continue;
                }
                radar.ticks_left -= 1;
                radar.next_look = radar.next_look.saturating_sub(1);
                if radar.next_look == 0 {
                    radar.found =
                        self.enemy_hero_within(team, radar.at, rules::units(rules::SCAN_RADIUS));
                    radar.next_look = rules::SCAN_LOOK_TICKS;
                }
            }
            self.scans.0[index] = Some(held);
        }
    }

    /// Whether a hero of the side against `side` stands within `radius` of `at`, its origin
    /// counted and the bound radius of its hull added to the reach, as Dota's finder takes it.
    /// Dota's finder asks for heroes with no leave for fog, invisibility, spell immunity or
    /// illusion, and none for the dead, those waiting to rise or the invulnerable: what stands
    /// and can be hurt is a hero here. The dead and those waiting to rise are not asked about: a
    /// hero stands nowhere from the moment it is laid down, and a blow that kills one buries it
    /// in the tick's damage phase, which no look falls inside. That is an invariant, asserted
    /// where the positions are read and proven by a test; the plain check beside it is for the
    /// product build, where the assertion is compiled out and hidden information fails closed:
    /// were the invariant ever broken, a dead hero would not be revealed. Couriers, creeps, wards and
    /// buildings are not heroes.
    fn enemy_hero_within(&self, side: Team, at: Vec2, radius: bota_proto::Fixed) -> bool {
        self.entities.iter().any(|entity| {
            self.is_hero(entity)
                && self
                    .team
                    .get(entity)
                    .is_some_and(|team| *team != side && *team != Team::Neutral)
                && !self
                    .stats
                    .get(entity)
                    .is_some_and(|stats| stats.invulnerable)
                && self.pos(entity).is_some_and(|pos| {
                    debug_assert!(
                        self.alive(entity),
                        "a hero with a place on the map is alive: the dead and those waiting to rise stand nowhere"
                    );
                    let bound = self
                        .hull
                        .get(entity)
                        .map_or(bota_proto::Fixed::ZERO, |hull| hull.bound);
                    self.alive(entity) && pos.within(at, radius + bound)
                })
        })
    }

    /// Dota's `-refresh` for Scan: the wait of each side that holds less than the most runs out
    /// now, for both sides whoever gave the cheat, so one charge arrives on the next tick and a
    /// fresh wait starts from it, what had passed of the old one lost; a side holding every
    /// charge is as it was.
    pub(crate) fn refresh_scans(&mut self) {
        let clock = self.clock();
        for index in 0..2 {
            let ledger = &mut self.scan_side_mut(index).ledger;
            if ledger.charges < rules::SCAN_MAX_CHARGES {
                ledger.next_charge = Some(clock);
            }
        }
    }

    /// What this tick tells of each side's Scan, restated in every batch to the side itself:
    /// its charges and the scans that look for it, from the first time it used or touched its
    /// Scan on. The other side is told nothing, whatever a scan found.
    pub(crate) fn scan_facts(&self, events: &mut Vec<Event>) {
        for team in [Team::Radiant, Team::Dire] {
            let Some(index) = side_index(team) else {
                continue;
            };
            if self.scans.0[index].is_none() {
                continue;
            }
            let held = self.scan_ledger(index);
            let mut radars: Vec<RadarView> = self.scans.0[index]
                .iter()
                .flat_map(|side| side.radars.iter().flatten())
                .map(|radar| RadarView {
                    at: radar.at,
                    ticks_left: radar.ticks_left,
                    found: radar.found,
                })
                .collect();
            radars.sort_by_key(|radar| radar.ticks_left);
            let told = EventVisibility::OneTeam(team);
            events.push(Event::Fact {
                kind: EventKind::Scan {
                    team,
                    charges: held.charges,
                    next_charge_in: held.next_in(self.clock()),
                    radars,
                },
                visible_to: told,
                gold_hidden_from: None,
                source_told_to: told,
            });
        }
    }
}
