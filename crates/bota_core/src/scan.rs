//! A side's Scan: how many charges it holds, and when the next one arrives.
//!
//! One ledger serves the whole side, whichever hero's player spends from it. Times are ticks
//! of the game clock, so the ledger of a side that never scanned is a plain function of the
//! clock ([`ScanLedger::untouched`]) and a client can show it before the server has anything
//! to tell.

use crate::rules::{SCAN_MAX_CHARGES, SCAN_RECHARGE_TICKS, Schedule};

/// What a side holds of its Scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanLedger {
    /// Charges held, at most [`SCAN_MAX_CHARGES`].
    pub charges: u8,
    /// The game-clock tick the next charge arrives; none while every charge is held.
    pub next_charge: Option<u32>,
}

impl ScanLedger {
    /// What the pregame leaves a side: no charge, and the first wait started.
    pub const fn pregame(schedule: Schedule) -> Self {
        Self {
            charges: 0,
            next_charge: Some(schedule.scan_first_charge()),
        }
    }

    /// What a side holds at game-clock tick `clock` when it never scanned and nothing else
    /// touched its Scan.
    pub fn untouched(schedule: Schedule, clock: u32) -> Self {
        let mut ledger = Self::pregame(schedule);
        ledger.catch_up(clock);
        ledger
    }

    /// Lets the charges arrive that are due by `clock`, one a wait, as many as have fallen due
    /// (a clock that jumped brings several at once). The wait for the next starts where the
    /// last one ended, and none runs once every charge is held.
    pub fn catch_up(&mut self, clock: u32) {
        assert!(
            self.charges <= SCAN_MAX_CHARGES,
            "more charges than Scan holds"
        );
        assert!(
            self.next_charge.is_none() || self.charges < SCAN_MAX_CHARGES,
            "a wait runs only below the most charges"
        );
        // Each round adds a charge and the loop ends at the most: SCAN_MAX_CHARGES rounds at
        // the very most.
        while let Some(due) = self.next_charge.filter(|due| clock >= *due) {
            self.charges += 1;
            self.next_charge = (self.charges < SCAN_MAX_CHARGES).then(|| due + SCAN_RECHARGE_TICKS);
        }
    }

    /// Spends one charge at `clock`, if there is one: the ledger must be caught up. A wait that
    /// is not running starts; one that is running goes on.
    #[must_use]
    pub fn spend(&mut self, clock: u32) -> bool {
        if self.charges == 0 {
            return false;
        }
        self.charges -= 1;
        self.next_charge.get_or_insert(clock + SCAN_RECHARGE_TICKS);
        true
    }

    /// Ticks until the next charge from `clock`; none while every charge is held.
    pub fn next_in(&self, clock: u32) -> Option<u32> {
        self.next_charge.map(|due| due.saturating_sub(clock))
    }
}
