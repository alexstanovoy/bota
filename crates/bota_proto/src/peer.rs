//! Peer identities and connection credentials, independent of simulation rules.

use serde::{Deserialize, Serialize};

/// A connection, as seen by the network layer.
///
/// Every connection gets a fresh one. A spectator has a `PlayerId` and no
/// [`crate::SlotId`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u32);

/// The secret that entitles its holder to one reserved seat, or to the
/// referee's connection.
///
/// Compared in constant time and never printed.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct SeatKey(pub [u8; 32]);

impl SeatKey {
    /// The key as 64 lowercase hex digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// A key from exactly 64 hex digits.
    pub fn from_hex(hex: &str) -> Option<SeatKey> {
        let digits = hex.as_bytes();
        if digits.len() != 64 {
            return None;
        }
        let mut key = [0u8; 32];
        for (byte, pair) in key.iter_mut().zip(digits.chunks(2)) {
            *byte = u8::from_str_radix(core::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(SeatKey(key))
    }
}

impl PartialEq for SeatKey {
    fn eq(&self, other: &Self) -> bool {
        let mut diff = 0u8;
        for (a, b) in self.0.iter().zip(&other.0) {
            diff |= a ^ b;
        }
        std::hint::black_box(diff) == 0
    }
}

impl Eq for SeatKey {}

impl core::fmt::Debug for SeatKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SeatKey(..)")
    }
}
