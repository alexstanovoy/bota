//! What a stream tells of each side's Scan, restated in every batch of events once a side has
//! used or touched it, and what the clock alone says before.

use bota_core::{ScanLedger, rules::Schedule, side_index};
use bota_proto::{EventKind, RadarView, Team};

/// One side's Scan as the client holds it: its charges, and the scans that are looking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanSide {
    /// Charges held and the game-clock tick the next arrives, as of now.
    pub ledger: ScanLedger,
    /// The scans looking, oldest first, as the last batch told them.
    pub radars: Vec<RadarView>,
}

/// What the last batches told of the Scan of each side, Radiant's first: a side no batch
/// named has never used its Scan and nothing touched it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanFacts {
    told: [Option<ScanSide>; 2],
}

impl ScanFacts {
    /// Takes in the Scan facts of a batch told at game-clock tick `game_tick`. A fact names the
    /// wait left; it is kept as the tick the next charge arrives, which holds until the next fact.
    pub(crate) fn take(&mut self, events: &[EventKind], game_tick: u32) {
        for event in events {
            let EventKind::Scan {
                team,
                charges,
                next_charge_in,
                radars,
            } = event
            else {
                continue;
            };
            let Some(index) = side_index(*team) else {
                continue;
            };
            self.told[index] = Some(ScanSide {
                ledger: ScanLedger {
                    charges: *charges,
                    next_charge: next_charge_in.map(|left| game_tick.saturating_add(left)),
                },
                radars: radars.clone(),
            });
        }
    }

    /// A side's Scan at game-clock tick `game_tick`: what was told, with the charges that have
    /// arrived since, or what the clock alone says of a side no batch named.
    pub(crate) fn of(&self, team: Team, schedule: Schedule, game_tick: u32) -> Option<ScanSide> {
        let index = side_index(team)?;
        Some(match &self.told[index] {
            Some(side) => {
                let mut side = side.clone();
                side.ledger.catch_up(game_tick);
                side
            }
            None => ScanSide {
                ledger: ScanLedger::untouched(schedule, game_tick),
                radars: Vec::new(),
            },
        })
    }
}

/// Whether the stream of `viewer` tells of the Scan of `team`: a player's own side's alone, a
/// spectator who sees all, both.
pub(crate) fn may_know(viewer: Option<Team>, team: Team) -> bool {
    viewer.is_none_or(|mine| mine == team)
}

/// Whether an event is a restated Scan fact rather than a thing that happened.
pub(crate) fn restated(event: &EventKind) -> bool {
    matches!(event, EventKind::Scan { .. })
}

#[cfg(test)]
mod tests {
    use bota_core::{ScanLedger, rules::Schedule};
    use bota_proto::{EventKind, Team};

    use super::{ScanFacts, may_know};

    #[test]
    fn a_player_may_know_only_its_own_sides_scan_and_a_spectator_both() {
        assert!(may_know(Some(Team::Radiant), Team::Radiant));
        assert!(!may_know(Some(Team::Radiant), Team::Dire));
        assert!(may_know(None, Team::Radiant) && may_know(None, Team::Dire));
    }

    #[test]
    fn a_told_ledger_goes_on_with_the_clock_and_an_untold_side_holds_what_the_clock_says() {
        let schedule = Schedule::of(900);
        let mut facts = ScanFacts::default();
        let fact = EventKind::Scan {
            team: Team::Dire,
            charges: 0,
            next_charge_in: Some(100),
            radars: Vec::new(),
        };
        facts.take(&[fact], 1000);
        let later = facts.of(Team::Dire, schedule, 1100).expect("a side");
        assert_eq!(later.ledger.charges, 1, "the charge arrived since");
        let untold = facts.of(Team::Radiant, schedule, 100).expect("a side");
        assert_eq!(untold.ledger, ScanLedger::pregame(schedule));
        assert_eq!(facts.of(Team::Neutral, schedule, 100), None);
    }
}
