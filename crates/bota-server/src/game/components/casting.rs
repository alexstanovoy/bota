//! A cast ordered and not yet begun.

use bota_proto::{AbilitySlot, ItemSlot, Target};

/// A cast ordered and not yet begun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingCast {
    /// One of the entity's abilities.
    Ability { slot: AbilitySlot, target: Target },
    /// One of the entity's items.
    Item { slot: ItemSlot, target: Target },
}

impl PendingCast {
    /// What it was aimed at.
    pub fn target(self) -> Target {
        match self {
            PendingCast::Ability { target, .. } | PendingCast::Item { target, .. } => target,
        }
    }
}
