//! A side's Scan: how many charges it holds, and when the next one arrives.
//!
//! One ledger serves the whole side, whichever hero's player spends from it. Times are ticks
//! of the game clock, so the ledger of a side that never scanned is a plain function of the
//! clock ([`ScanLedger::untouched`]) and a client can show it before the server has anything
//! to tell.

use crate::rules::{SCAN_MAX_CHARGES, SCAN_RECHARGE_TICKS, Schedule};
use crate::{ChargeRule, Charges};

/// How a side's Scan charges come back.
pub const SCAN_CHARGES: ChargeRule = ChargeRule {
    max: SCAN_MAX_CHARGES,
    recharge: SCAN_RECHARGE_TICKS,
};

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

    /// Lets the charges arrive that are due by `clock` ([`Charges::catch_up`]).
    pub fn catch_up(&mut self, clock: u32) {
        let mut held = self.held();
        held.catch_up(clock, SCAN_CHARGES);
        self.keep(held);
    }

    /// Spends one charge at `clock`, if there is one ([`Charges::spend`]): the ledger must be
    /// caught up.
    #[must_use]
    pub fn spend(&mut self, clock: u32) -> bool {
        let mut held = self.held();
        let spent = held.spend(clock, SCAN_CHARGES);
        self.keep(held);
        spent
    }

    /// Ticks until the next charge from `clock`; none while every charge is held.
    pub fn next_in(&self, clock: u32) -> Option<u32> {
        self.held().next_in(clock)
    }

    const fn held(&self) -> Charges {
        Charges {
            charges: self.charges,
            next_charge: self.next_charge,
        }
    }

    fn keep(&mut self, held: Charges) {
        self.charges = held.charges;
        self.next_charge = held.next_charge;
    }
}
