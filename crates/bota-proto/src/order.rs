//! What a participant asks its hero to do.
//!
//! An order is an intent. The server validates it, may reject it, and decides
//! what actually happens; the result shows up in the next snapshot and in
//! [`EventKind`](crate::EventKind).
//!
//! At most one order per seat survives per tick, and the last one submitted
//! wins. There is no shift-queue in v0.1.

use crate::{AbilitySlot, EntityId, ItemId, ItemSlot, Vec2};
use serde::{Deserialize, Serialize};

/// Where an order is aimed.
///
/// Which variant is legal depends on the order carrying it. A mismatch is
/// rejected with
/// [`RejectReason::WrongTargetKind`](crate::RejectReason::WrongTargetKind).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// At nothing: the order works on the unit itself, or where it stands.
    None,
    /// At a position on the ground.
    Pos(Vec2),
    /// At a live entity. Must be visible to the issuing team.
    Unit(EntityId),
}

/// A single instruction from a participant to its own hero.
///
/// A target the issuing team cannot currently see is rejected with
/// [`RejectReason::UnknownTarget`](crate::RejectReason::UnknownTarget).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Order {
    /// Go somewhere, ignoring enemies on the way.
    ///
    /// Aimed at nothing it cancels the current order and stands still. Aimed
    /// at a position it walks there. Aimed at a unit it follows that unit; a
    /// plain follow calls no enemy creeps or towers on or off.
    Move {
        /// Where to go: nothing to stand still, a position to walk to, a
        /// unit to follow.
        target: Target,
    },
    /// Fight whatever the order is aimed at.
    ///
    /// Aimed at nothing it stands still, but attacks anything that comes into
    /// range. Aimed at a position it walks there, stopping to attack enemies
    /// encountered on the way. Aimed at a unit it attacks that unit, following
    /// it if it moves out of range; against a friendly unit this is a follow,
    /// turning into a deny once the unit is low enough to allow one, and
    /// either way an order aimed at a unit calls off any enemy creeps and
    /// towers currently aggroed on the issuer.
    Attack {
        /// What to fight: nothing to hold position, a position to
        /// attack-move to, a unit to attack.
        target: Target,
    },
    /// Cast one of the hero's abilities.
    Cast {
        /// Which of the four ability slots to cast.
        slot: AbilitySlot,
        /// What the ability is aimed at.
        target: Target,
    },
    /// Activate an item in the inventory.
    Use {
        /// Which inventory slot holds the item.
        slot: ItemSlot,
        /// What the item is aimed at.
        target: Target,
    },
    /// Lay an item out of the bag: on the ground, or into an ally's hands.
    ///
    /// Aimed at a position it lands there, aimed at nothing it lands
    /// underfoot, and aimed at an allied unit with a bag it goes into that
    /// bag's first free slot. The unit walks into reach first when it has to.
    Put {
        /// Which bag slot gives the item up. Stash slots take no part.
        slot: ItemSlot,
        /// Where the item goes.
        target: Target,
    },
    /// Take an item lying on the ground into the first free bag slot.
    ///
    /// The unit walks over to it first when it has to. Any unit with a bag may
    /// take any ground item, whoever dropped it.
    Take {
        /// The ground item to take. Must be [`Target::Unit`]; anything else
        /// is rejected with
        /// [`RejectReason::WrongTargetKind`](crate::RejectReason::WrongTargetKind).
        target: Target,
    },
    /// Buy an item. Legal only while standing in the fountain area.
    Buy {
        /// What to buy.
        item: ItemId,
    },
    /// Sell an item from the inventory for part of its cost.
    ///
    /// Away from the shop this marks the stack for sale instead, and a second
    /// order on the same slot unmarks it. A marked stack is sold the moment it
    /// reaches the shop — carried there, delivered by courier, or put in the
    /// stash.
    Sell {
        /// Which inventory slot to empty.
        slot: ItemSlot,
    },
    /// Move an item between two slots, swapping whatever is in the way.
    ///
    /// Stash slots take part only while standing in the home shop area.
    Swap {
        /// The slot being moved from.
        from: ItemSlot,
        /// The slot being moved to.
        to: ItemSlot,
    },
    /// Spend a skill point on an ability.
    Learn {
        /// Which of the four ability slots to level.
        slot: AbilitySlot,
    },
}
