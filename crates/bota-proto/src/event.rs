//! Things that happened during one tick.
//!
//! A team is sent only the events it may know of.

use crate::{AbilityId, EntityId, ItemId, SlotId, Team};
use serde::{Deserialize, Serialize};

/// How a chunk of damage is reduced before it is applied.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DamageKind {
    /// Reduced by armor.
    Physical,
    /// Reduced by magic resistance.
    Magical,
    /// Not reduced by anything.
    Pure,
}

/// A single thing that happened on one tick.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A unit took damage.
    Damaged {
        /// Who dealt it, if anyone; may name a unit that has since died.
        source: Option<EntityId>,
        /// Who took it.
        target: EntityId,
        /// Health actually lost, after armor and resistance.
        amount: i32,
        /// Which reduction applied.
        kind: DamageKind,
        /// Whether this hit was a critical strike.
        crit: bool,
    },
    /// An attack did not land: the target evaded it, or it was thrown
    /// uphill and missed.
    Missed {
        /// Who swung; may name a unit that has since died.
        source: Option<EntityId>,
        /// Who it was swung at.
        target: EntityId,
    },
    /// A unit was mended by somebody's hand: an item drunk or a charge
    /// spent. Passive regeneration and the fountain are not told of.
    Healed {
        /// Who mended it.
        source: Option<EntityId>,
        /// Who was mended.
        target: EntityId,
        /// Health the mending is good for: no more than it holds, no more
        /// than was missing when it began. A mend paid out over time may
        /// still be cut short by a blow.
        amount: i32,
        /// Mana it restores alongside, on the same counting.
        mana: i32,
    },
    /// A unit died.
    Died {
        /// The unit that died.
        unit: EntityId,
        /// Who landed the killing blow, if a unit did.
        killer: Option<EntityId>,
        /// Whether the killer was on the same team, making this a deny.
        denied: bool,
        /// Gold paid to the seat of the hero that struck last. Zero for a
        /// deny, or when no hero struck last.
        gold: i32,
    },
    /// A unit's ability took effect.
    ///
    /// A cast is told at the moment of effect, not when the order was issued.
    AbilityCast {
        /// Who cast it.
        caster: EntityId,
        /// Which ability.
        ability: AbilityId,
    },
    /// A hero gained a level. Not sent for a level gained while it is dead.
    LevelUp {
        /// Which hero.
        unit: EntityId,
        /// The level just reached.
        level: u8,
    },
    /// A seat bought an item, or was handed one by a cheat. Told to its own
    /// team alone.
    ItemBought {
        /// Which seat bought it.
        slot: SlotId,
        /// What was bought.
        item: ItemId,
    },
    /// A building was destroyed.
    StructureDestroyed {
        /// Which building.
        unit: EntityId,
        /// Which team lost it.
        team: Team,
    },
}
