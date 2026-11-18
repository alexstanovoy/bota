//! Scan: each side's charges, and a scan that is looking.

use bota_core::ScanLedger;

use crate::engine::{Fnv, StateHash, state_hash};

/// The tag a side's Scan ledgers go into the world fingerprint under.
const SCAN_BOOK_HASH_TAG: u8 = 0xB6;

/// Both sides' Scan charges, Radiant's first. A side's ledger is absent until its Scan is used
/// or touched: until then it is [`ScanLedger::untouched`] by the game clock, and the fingerprint
/// of a match nobody scans in carries nothing of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanBook(pub [Option<ScanLedger>; 2]);

impl StateHash for ScanBook {
    fn state_hash(&self, fnv: &mut Fnv) {
        if self.0.iter().all(Option::is_none) {
            return;
        }
        fnv.u8(SCAN_BOOK_HASH_TAG);
        for ledger in &self.0 {
            fnv.some(ledger.is_some());
            if let Some(ledger) = ledger {
                ledger.charges.state_hash(fnv);
                ledger.next_charge.state_hash(fnv);
            }
        }
    }
}

#[cfg(test)]
impl crate::engine::Specimen for ScanBook {
    fn low() -> Self {
        ScanBook::default()
    }

    fn high() -> Self {
        let held = Some(ScanLedger {
            charges: 1,
            next_charge: Some(2),
        });
        ScanBook([held, held])
    }
}

/// What a scan that is looking holds, on the entity placed at the point it looks around, which
/// is of the side that used it and carries an [`Expiry`](crate::game::Expiry) of the scan's
/// duration: Dota's radar thinker, a body-less unit with `modifier_radar_thinker`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Radar {
    /// What the latest look found: whether an enemy hero stood in the area.
    pub found: bool,
    /// Ticks until the next look.
    pub next_look: u32,
}

state_hash!(Radar { found, next_look });
