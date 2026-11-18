//! Scan: a side's shared charges, and the scans that look for enemy heroes.
//!
//! Dota's team ability. One ledger of charges serves a whole side. Using it stands a body-less
//! thinker at the point, which looks at once and then every second for eight seconds, and the
//! side is told after every look whether an enemy hero stood in the area, and nothing else.

use bota_core::{ScanLedger, rules, side_index};
use bota_proto::{Angle, EventKind, MAX_SCAN_RADARS, RadarView, RejectReason, SlotId, Team, Vec2};

use crate::engine::Entity;
use crate::game::{Event, EventVisibility, Expiry, Radar, World};

impl World {
    /// What a side's Scan holds now: its ledger as the clock leaves it, or what the clock alone
    /// says while nothing has touched it.
    pub fn scan_ledger(&self, index: usize) -> ScanLedger {
        let clock = self.clock();
        match self.scans.0[index] {
            Some(mut held) => {
                held.catch_up(clock);
                held
            }
            None => ScanLedger::untouched(self.schedule(), clock),
        }
    }

    /// The ledger of a side, taken out of the clock's hands: from here on the match carries it.
    fn scan_ledger_mut(&mut self, index: usize) -> &mut ScanLedger {
        let held = self.scan_ledger(index);
        self.scans.0[index].insert(held)
    }

    /// Whether a seat's side may use its Scan now: it holds a charge, and no more than
    /// [`MAX_SCAN_RADARS`] of its scans are looking (a bound only a cheat's refresh can reach).
    /// Nothing else is asked, as in Dota: not the hero's life, not its mana, not where the point is.
    pub fn check_scan(&self, slot: SlotId) -> Result<(), RejectReason> {
        let seat = self.seat(slot).ok_or(RejectReason::HeroDead)?;
        let team = seat.team;
        let index = side_index(team).ok_or(RejectReason::NotYourUnit)?;
        if self.scan_ledger(index).charges == 0 || self.scans_looking(team) >= MAX_SCAN_RADARS {
            return Err(RejectReason::NoScanCharge);
        }
        Ok(())
    }

    /// How many scans of a side are looking now.
    fn scans_looking(&self, team: Team) -> usize {
        if self.radar.is_empty() {
            return 0;
        }
        self.entities
            .iter()
            .filter(|&entity| self.radar.contains(entity) && self.team.get(entity) == Some(&team))
            .count()
    }

    /// Uses the Scan of a seat's side at a point, if it holds a charge: spends it, stands the
    /// thinker there and has it look once at once.
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
        assert!(
            self.scan_ledger_mut(index).spend(clock),
            "a side that holds a charge spends it"
        );
        let thinker = self.spawn_placed(at, Angle::default(), team);
        self.expiry.insert(
            thinker,
            Expiry {
                ticks_left: rules::SCAN_THINKER_TICKS,
            },
        );
        self.radar.insert(
            thinker,
            Radar {
                found: false,
                next_look: 0,
            },
        );
        self.look_around(thinker);
    }

    /// Lets the charges arrive that are due, and has every scan that is due look again.
    pub(crate) fn tick_scans(&mut self) {
        let clock = self.clock();
        for held in self.scans.0.iter_mut().flatten() {
            held.catch_up(clock);
        }
        if self.radar.is_empty() {
            return;
        }
        let thinkers: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&entity| self.radar.contains(entity))
            .collect();
        for thinker in thinkers {
            let Some(radar) = self.radar.get_mut(thinker) else {
                continue;
            };
            radar.next_look = radar.next_look.saturating_sub(1);
            if radar.next_look == 0 {
                self.look_around(thinker);
            }
        }
    }

    /// One look of a scan: whether an enemy hero stands in its area now, which replaces what
    /// the last look found.
    fn look_around(&mut self, thinker: Entity) {
        let (Some(side), Some(at)) = (self.team.get(thinker).copied(), self.pos(thinker)) else {
            return;
        };
        let found = self.enemy_hero_within(side, at, rules::units(rules::SCAN_RADIUS));
        let Some(radar) = self.radar.get_mut(thinker) else {
            return;
        };
        radar.found = found;
        radar.next_look = rules::SCAN_LOOK_TICKS;
    }

    /// Whether a hero of the side against `side` stands within `radius` of `at`, its origin
    /// counted and the bound radius of its hull added to the reach, as Dota's finder takes it. Dota's finder asks for heroes with no leave for fog, invisibility, spell
    /// immunity or illusion, and none for the dead, those waiting to rise or the invulnerable:
    /// what stands and can be hurt is a hero here. Couriers, creeps, wards and buildings are
    /// not heroes.
    fn enemy_hero_within(&self, side: Team, at: Vec2, radius: bota_proto::Fixed) -> bool {
        self.entities.iter().any(|entity| {
            self.is_hero(entity)
                && self
                    .team
                    .get(entity)
                    .is_some_and(|team| *team != side && *team != Team::Neutral)
                && self.alive(entity)
                && !self
                    .stats
                    .get(entity)
                    .is_some_and(|stats| stats.invulnerable)
                && self.pos(entity).is_some_and(|pos| {
                    let bound = self
                        .hull
                        .get(entity)
                        .map_or(bota_proto::Fixed::ZERO, |hull| hull.bound);
                    pos.within(at, radius + bound)
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
            let held = self.scan_ledger_mut(index);
            if held.charges < rules::SCAN_MAX_CHARGES {
                held.next_charge = Some(clock);
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
            let mut radars: Vec<(u32, RadarView)> = self
                .entities
                .iter()
                .filter(|&entity| self.team.get(entity) == Some(&team))
                .filter_map(|entity| {
                    let radar = self.radar.get(entity)?;
                    let ticks_left = self.expiry.get(entity)?.ticks_left.saturating_sub(1);
                    Some((
                        ticks_left,
                        RadarView {
                            at: self.pos(entity)?,
                            ticks_left,
                            found: radar.found,
                        },
                    ))
                })
                .collect();
            radars.sort_by_key(|(ticks_left, _)| *ticks_left);
            let told = EventVisibility::OneTeam(team);
            events.push(Event::Fact {
                kind: EventKind::Scan {
                    team,
                    charges: held.charges,
                    next_charge_in: held.next_in(self.clock()),
                    radars: radars.into_iter().map(|(_, view)| view).collect(),
                },
                visible_to: told,
                gold_hidden_from: None,
                source_told_to: told,
            });
        }
    }
}
