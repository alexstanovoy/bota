//! An ability begun and waiting on its facing.

use bota_proto::{AbilitySlot, Target};

/// A cast ordered and not yet started. Absent when nothing is pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingCast {
    /// Which of the entity's abilities.
    pub slot: AbilitySlot,
    /// What it was aimed at.
    pub target: Target,
}
