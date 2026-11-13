//! A seat's buyback.

use crate::engine::{Fnv, StateHash};

/// The tag a seat's buyback goes into the world fingerprint under.
const BUYBACK_HASH_TAG: u8 = 0xB7;

/// What a seat's buybacks have left: its cooldown, the respawn time its
/// hero's next death owes, and when it last bought back. A seat that never
/// bought back and whose buyback no cheat touched holds the default, and the
/// fingerprint of a match nobody buys back in carries nothing of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buyback {
    /// The tick the seat may buy back from: Dota's `m_fBuybackCooldown`, a
    /// time and no wait. Zero before any buyback.
    pub ready_at: u32,
    /// Ticks the hero's next death that is no reincarnation lasts longer
    /// (Dota's `m_flRespawnTimePenalty`).
    pub respawn_penalty: u32,
    /// The tick of the latest buyback (Dota's `m_flLastBuybackTime`).
    pub last: Option<u32>,
}

impl StateHash for Buyback {
    fn state_hash(&self, fnv: &mut Fnv) {
        if *self == Buyback::default() {
            return;
        }
        fnv.u8(BUYBACK_HASH_TAG);
        self.ready_at.state_hash(fnv);
        self.respawn_penalty.state_hash(fnv);
        self.last.state_hash(fnv);
    }
}

#[cfg(test)]
impl crate::engine::Specimen for Buyback {
    fn low() -> Self {
        Buyback::default()
    }

    fn high() -> Self {
        Buyback {
            ready_at: 1,
            respawn_penalty: 2,
            last: Some(3),
        }
    }
}
