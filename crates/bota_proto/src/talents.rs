//! The learned talent slots used by hero rules.

use crate::TalentSlot;
use serde::{Deserialize, Serialize};

/// A set of a hero's talent slots: bit `n` holds [`TalentSlot`]`(n)`.
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct Talents {
    /// The raw bits.
    pub bits: u8,
}

impl Talents {
    /// Whether the set holds a slot.
    pub const fn has(self, slot: TalentSlot) -> bool {
        slot.0 < TalentSlot::COUNT && self.bits & (1 << slot.0) != 0
    }

    /// The set with a slot added.
    pub const fn with(self, slot: TalentSlot) -> Talents {
        assert!(slot.0 < TalentSlot::COUNT);
        Talents {
            bits: self.bits | (1 << slot.0),
        }
    }
}
