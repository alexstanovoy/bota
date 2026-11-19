//! A side's Glyph of Fortification.

use bota_core::{Charges, rules};

use crate::engine::state_hash;

/// The first tier one tower of the side has fallen and refreshed the Glyph.
pub const GLYPH_TIER_ONE: u8 = 1;
/// The first tier two tower of the side has fallen and refreshed the Glyph.
pub const GLYPH_TIER_TWO: u8 = 2;
/// The first melee barracks of the side has fallen and refreshed the Glyph.
pub const GLYPH_MELEE_RAX: u8 = 4;

/// One side's Glyph of Fortification: its charges and their wait (Dota 7.42: two, six minutes
/// each), the wait between two uses, its protection, and the refreshes it has had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// Charges held as the side last touched them; [`Glyph::held`] catches them up to a tick.
    pub charges: u8,
    /// The game-clock tick the next charge arrives, as the side last touched them; none while
    /// every charge is held.
    pub next_charge: Option<u32>,
    /// The tick the side may use a charge from, seven seconds after the last use.
    pub usable_at: u32,
    /// Until this tick its structures take no damage.
    pub structures_until: u32,
    /// The refreshes it has had, as the `GLYPH_*` bits: each building falls
    /// to refresh it once.
    pub refreshed: u8,
}

impl Glyph {
    /// A Glyph not yet used on a match with schedule `schedule`: no charge, the first arriving
    /// at the first moment Dota allows.
    pub const fn fresh(schedule: rules::Schedule) -> Glyph {
        let first = Charges::first_at(schedule.glyph_first_use());
        Glyph {
            charges: first.charges,
            next_charge: first.next_charge,
            usable_at: 0,
            structures_until: 0,
            refreshed: 0,
        }
    }

    /// Its charges at tick `now`, those due by then arrived.
    pub fn held(&self, now: u32) -> Charges {
        let mut held = Charges {
            charges: self.charges,
            next_charge: self.next_charge,
        };
        held.catch_up(now, rules::GLYPH_CHARGES);
        held
    }

    /// Keeps `held` as its charges.
    pub fn keep(&mut self, held: Charges) {
        self.charges = held.charges;
        self.next_charge = held.next_charge;
    }

    /// Ticks from `now` until the side may use it: none while it holds a charge and the wait
    /// between uses is over; else the longer of that wait and the next charge's.
    pub fn ready_after(&self, now: u32) -> u32 {
        let between = self.usable_at.saturating_sub(now);
        let held = self.held(now);
        if held.charges > 0 {
            between
        } else {
            between.max(held.next_in(now).unwrap_or(0))
        }
    }
}

state_hash!(Glyph {
    charges,
    next_charge,
    usable_at,
    structures_until,
    refreshed
});
