use bota_proto::{AbilitySlot, ItemSlot, Target};

use crate::engine::{Fnv, StateHash};

/// A cast ordered and not yet begun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingCast {
    /// One of the entity's abilities.
    Ability { slot: AbilitySlot, target: Target },
    /// One of the entity's items.
    Item { slot: ItemSlot, target: Target },
}

impl PendingCast {
    pub fn target(self) -> Target {
        match self {
            PendingCast::Ability { target, .. } | PendingCast::Item { target, .. } => target,
        }
    }
}

impl StateHash for PendingCast {
    fn state_hash(&self, fnv: &mut Fnv) {
        match self {
            PendingCast::Ability { slot, target } => {
                fnv.u8(0);
                slot.state_hash(fnv);
                target.state_hash(fnv);
            }
            PendingCast::Item { slot, target } => {
                fnv.u8(1);
                slot.state_hash(fnv);
                target.state_hash(fnv);
            }
        }
    }
}

#[cfg(test)]
impl crate::engine::Specimen for PendingCast {
    fn low() -> Self {
        PendingCast::Ability {
            slot: crate::engine::Specimen::low(),
            target: crate::engine::Specimen::low(),
        }
    }

    fn high() -> Self {
        PendingCast::Item {
            slot: crate::engine::Specimen::high(),
            target: crate::engine::Specimen::high(),
        }
    }
}
