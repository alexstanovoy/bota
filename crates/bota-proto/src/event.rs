//! Things that happened during one tick.
//!
//! A snapshot carries state, an event carries what occurred. Anything
//! instantaneous, such as a hit that took a unit from full health to dead,
//! appears here and nowhere else.
//!
//! The server drops the events a team may not see before sending.

use crate::{AbilityId, EntityId, ItemId, SlotId, Team};
use serde::{Deserialize, Serialize};

/// How a chunk of damage is reduced before it is applied.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DamageKind {
    /// Reduced by armor. Dealt by attacks and most melee abilities.
    Physical,
    /// Reduced by magic resistance. Dealt by most abilities.
    Magical,
    /// Not reduced by anything.
    Pure,
}

/// A single thing that happened on one tick.
///
/// Used by the client for damage numbers, sounds and the kill feed, and by a bot
/// to notice what a snapshot does not show.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A unit took damage.
    Damaged {
        /// Who dealt it. Absent for environmental damage such as the fountain.
        source: Option<EntityId>,
        /// Who took it.
        target: EntityId,
        /// Health actually lost, after armor and resistance.
        amount: i32,
        /// Which reduction applied.
        kind: DamageKind,
        /// Whether this hit was a critical strike. Reported here and nowhere
        /// else.
        crit: bool,
    },
    /// A unit was healed.
    Healed {
        /// Who healed it. Absent for passive regeneration.
        source: Option<EntityId>,
        /// Who was healed.
        target: EntityId,
        /// Health actually restored, after any healing cap.
        amount: i32,
    },
    /// A unit died.
    Died {
        /// The unit that died.
        unit: EntityId,
        /// Who landed the killing blow, if a unit did.
        killer: Option<EntityId>,
        /// Whether the killer was on the same team, making this a deny.
        denied: bool,
        /// Gold the killing side was paid for it. Nought for a deny, or when
        /// nothing with a seat struck last.
        gold: i32,
    },
    /// A hero finished a cast and the ability took effect.
    ///
    /// Emitted at the moment of effect, not when the order was issued.
    AbilityCast {
        /// Who cast it.
        caster: EntityId,
        /// Which ability.
        ability: AbilityId,
    },
    /// A hero gained a level.
    LevelUp {
        /// Which hero.
        unit: EntityId,
        /// The level just reached.
        level: u8,
    },
    /// A hero bought an item.
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
