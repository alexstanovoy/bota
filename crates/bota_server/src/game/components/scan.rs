//! Scan: each side's charges, and the scans that are looking.

use bota_core::ScanLedger;
use bota_proto::{MAX_SCAN_RADARS, Vec2};

use crate::engine::{Fnv, StateHash, state_hash};

/// The tag a side's Scan records go into the world fingerprint under.
const SCAN_BOOK_HASH_TAG: u8 = 0xB6;

/// A scan that is looking: Dota's radar thinker. It is a side's own record of a point and a clock
/// and nothing a unit meets, so it is no entity of the world: it takes no handle from the
/// allocator, and nothing a scan does can move what the other side is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Radar {
    /// The centre of the area it looks in.
    pub at: Vec2,
    /// Ticks until it ends; one more tick, at zero, lets it go.
    pub ticks_left: u32,
    /// Ticks until the next look.
    pub next_look: u32,
    /// What the latest look found: whether an enemy hero stood in the area.
    pub found: bool,
}

state_hash!(Radar {
    at,
    ticks_left,
    next_look,
    found
});

/// What a side holds of its Scan once it has used it or a cheat touched it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanSide {
    pub ledger: ScanLedger,
    /// The scans that are looking, in the places they were set up in.
    pub radars: [Option<Radar>; MAX_SCAN_RADARS],
}

impl ScanSide {
    /// A side whose Scan holds `ledger` and looks nowhere.
    pub const fn new(ledger: ScanLedger) -> Self {
        Self {
            ledger,
            radars: [None; MAX_SCAN_RADARS],
        }
    }

    /// How many of its scans are looking.
    pub fn looking(&self) -> usize {
        self.radars.iter().flatten().count()
    }
}

impl StateHash for ScanSide {
    fn state_hash(&self, fnv: &mut Fnv) {
        self.ledger.charges.state_hash(fnv);
        self.ledger.next_charge.state_hash(fnv);
        self.radars.state_hash(fnv);
    }
}

/// Both sides' Scan, Radiant's first. A side's is absent until it is used or touched: until then
/// the side's ledger is [`ScanLedger::untouched`] by the game clock and it looks nowhere, and the
/// fingerprint of a match nobody scans in carries nothing of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanBook(pub [Option<ScanSide>; 2]);

impl StateHash for ScanBook {
    fn state_hash(&self, fnv: &mut Fnv) {
        if self.0.iter().all(Option::is_none) {
            return;
        }
        fnv.u8(SCAN_BOOK_HASH_TAG);
        self.0.state_hash(fnv);
    }
}

#[cfg(test)]
impl crate::engine::Specimen for ScanBook {
    fn low() -> Self {
        ScanBook::default()
    }

    fn high() -> Self {
        let mut held = ScanSide::new(ScanLedger {
            charges: 1,
            next_charge: Some(2),
        });
        held.radars[0] = Some(Radar {
            at: Vec2::from_ints(3, 4),
            ticks_left: 5,
            next_look: 6,
            found: true,
        });
        ScanBook([Some(held), Some(held)])
    }
}
