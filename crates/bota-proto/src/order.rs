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

/// What an ability or item is being pointed at.
///
/// Which variant is legal depends on the ability being cast. A mismatch is
/// rejected with
/// [`RejectReason::WrongTargetKind`](crate::RejectReason::WrongTargetKind).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrderTarget {
    /// Cast on self or with no target at all.
    None,
    /// Cast at a position on the ground.
    Point {
        /// Where on the map the cast is aimed.
        pos: Vec2,
    },
    /// Cast at a specific unit.
    Unit {
        /// The unit being aimed at. Must be visible to the caster's team.
        target: EntityId,
    },
}

/// A single instruction from a participant to its own hero.
///
/// A target the issuing team cannot currently see is rejected with
/// [`RejectReason::UnknownTarget`](crate::RejectReason::UnknownTarget).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Order {
    /// Cancel the current order and stand still.
    Stop,
    /// Stand still, but attack anything that comes into range.
    HoldPosition,
    /// Walk to a position, ignoring enemies on the way.
    Move {
        /// Destination.
        pos: Vec2,
    },
    /// Walk to a position, stopping to attack enemies encountered on the way.
    AttackMove {
        /// Destination.
        pos: Vec2,
    },
    /// Attack a specific unit, following it if it moves out of range.
    ///
    /// Against a friendly unit this is a follow, turning into a deny once the
    /// unit is low enough to allow one. Either way the order calls off any
    /// enemy creeps and towers currently aggroed on the issuer.
    AttackUnit {
        /// The unit to attack.
        target: EntityId,
    },
    /// Cast one of the hero's abilities.
    CastAbility {
        /// Which of the four ability slots to cast.
        slot: AbilitySlot,
        /// What the ability is aimed at.
        target: OrderTarget,
    },
    /// Activate an item in the inventory.
    UseItem {
        /// Which inventory slot holds the item.
        slot: ItemSlot,
        /// What the item is aimed at.
        target: OrderTarget,
    },
    /// Move an item between two slots, swapping whatever is in the way.
    ///
    /// Stash slots take part only while standing in the home shop area.
    MoveItem {
        /// The slot being moved from.
        from: ItemSlot,
        /// The slot being moved to.
        to: ItemSlot,
    },
    /// Spend a skill point on an ability.
    LevelUpAbility {
        /// Which of the four ability slots to level.
        slot: AbilitySlot,
    },
    /// Buy an item. Legal only while standing in the fountain area.
    BuyItem {
        /// What to buy.
        item: ItemId,
    },
    /// Sell an item from the inventory for part of its cost.
    ///
    /// Away from the shop this marks the stack for sale instead, and a second
    /// order on the same slot unmarks it. A marked stack is sold the moment it
    /// reaches the shop — carried there, delivered by courier, or put in the
    /// stash.
    SellItem {
        /// Which inventory slot to empty.
        slot: ItemSlot,
    },
    /// Lay an item out of the bag: on the ground, or into an ally's hands.
    ///
    /// Aimed at a point it lands there, aimed at nothing it lands underfoot,
    /// and aimed at an allied unit with a bag it goes into that bag's first
    /// free slot. The unit walks into reach first when it has to.
    PutItem {
        /// Which bag slot gives the item up. Stash slots take no part.
        slot: ItemSlot,
        /// Where the item goes.
        target: OrderTarget,
    },
    /// Take an item lying on the ground into the first free bag slot.
    ///
    /// The unit walks over to it first when it has to. Any unit with a bag may
    /// take any ground item, whoever dropped it.
    TakeItem {
        /// The ground item to take. Must be visible to the issuing team.
        target: EntityId,
    },
}
