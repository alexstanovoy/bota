//! What a unit is walking to do with an item.

use bota_proto::Vec2;

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
    /// Taking a ground item into the first free bag slot.
    Take {
        /// The ground item to take.
        loot: Entity,
    },
}
