use bota_proto::Vec2;

use crate::engine::{Fnv, StateHash};
use crate::game::Entity;

/// An item errand a unit carries out once it is close enough.
///
/// It outlives the tick it was given in, and any later order calls it off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handling {
    /// Laying what sits in a bag slot on the ground.
    PutAt {
        /// The bag slot giving the item up.
        slot: usize,
        /// The spot it lands on.
        pos: Vec2,
    },
    /// Handing what sits in a bag slot to another unit's bag.
    PutTo {
        /// The bag slot giving the item up.
        slot: usize,
        /// The unit whose bag it goes to.
        to: Entity,
    },
    /// Taking a ground item into the first free bag slot, or walking to a
    /// rune until it is taken.
    Take {
        /// The ground item or the rune to take.
        loot: Entity,
    },
    /// Walking into attack range of a rune to destroy it.
    Deny {
        /// The rune to destroy.
        rune: Entity,
    },
}

impl StateHash for Handling {
    fn state_hash(&self, fnv: &mut Fnv) {
        match self {
            Handling::PutAt { slot, pos } => {
                fnv.u8(0);
                slot.state_hash(fnv);
                pos.state_hash(fnv);
            }
            Handling::PutTo { slot, to } => {
                fnv.u8(1);
                slot.state_hash(fnv);
                to.state_hash(fnv);
            }
            Handling::Take { loot } => {
                fnv.u8(2);
                loot.state_hash(fnv);
            }
            Handling::Deny { rune } => {
                fnv.u8(3);
                rune.state_hash(fnv);
            }
        }
    }
}

#[cfg(test)]
impl crate::engine::Specimen for Handling {
    fn low() -> Self {
        Handling::Take {
            loot: crate::engine::Specimen::low(),
        }
    }

    fn high() -> Self {
        Handling::PutTo {
            slot: crate::engine::Specimen::high(),
            to: crate::engine::Specimen::high(),
        }
    }
}
