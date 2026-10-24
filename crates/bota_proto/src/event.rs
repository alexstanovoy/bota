//! Things that happened during one tick.
//!
//! A team is sent only the events it may know of, as the
//! [contract](crate::contract) says: those of the units it sees, and of who
//! dealt them only a dealer it sees.

use crate::{AbilityId, EntityId, Inflictor, ItemId, SlotId, Team, Vec2};
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
        /// `None` to a side that does not see the dealer.
        source: Option<EntityId>,
        /// Who took it.
        target: EntityId,
        /// Health actually lost, after armor and resistance.
        amount: i32,
        /// Which reduction applied.
        kind: DamageKind,
        /// The ability or item that dealt it. Absent for an attack's own
        /// blow.
        inflictor: Option<Inflictor>,
    },
    /// An attack did not land: the target evaded it, or it was thrown
    /// uphill and missed.
    Missed {
        /// Who swung; may name a unit that has since died. `None` to a side
        /// that does not see the swinger.
        source: Option<EntityId>,
        /// Who it was swung at.
        target: EntityId,
    },
    /// A unit was mended by somebody's hand: an item drunk or a charge
    /// spent. Passive regeneration and the fountain are not told of.
    Healed {
        /// Who mended it. `None` to a side that does not see the mender.
        source: Option<EntityId>,
        /// Who was mended.
        target: EntityId,
        /// Health the mending is good for: no more than it holds, no more
        /// than was missing when it began. A mend paid out over time may
        /// still be cut short by a blow.
        amount: i32,
        /// Mana it restores alongside, on the same counting.
        mana: i32,
        /// The item or ability that mends it.
        inflictor: Option<Inflictor>,
    },
    /// A unit died.
    Died {
        /// The unit that died.
        unit: EntityId,
        /// Who landed the killing blow, if a unit did. `None` to a side that
        /// does not see the killer, unless the kill feed names it: a hero
        /// that brought down a hero, a tower or a barracks is told to both
        /// sides.
        killer: Option<EntityId>,
        /// Whether the killer was on the same team, making this a deny.
        denied: bool,
        /// Retired: always zero. `bounty` tells it.
        gold: i32,
        /// Gold paid to the seat of the hero that struck last: the bounty
        /// drawn for this kill, a flagbearer's bonus and a tower's team
        /// bounty on top. On a deny only a tower's half team bounty. Zero
        /// when no hero struck last. Absent to the side that did not earn a
        /// non-hero kill: another's income is its own. A hero kill's gold
        /// is public, as Dota's kill feed tells it.
        bounty: Option<i32>,
    },
    /// A unit's ability took effect.
    ///
    /// A cast is told at the moment of effect, not when the order was issued.
    AbilityCast {
        /// Who cast it.
        caster: EntityId,
        ability: AbilityId,
    },
    /// A hero gained a level. Not sent for a level gained while it is dead,
    /// and told to the other side only when it sees the hero.
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
    /// A teleport channel ended, told on the tick it did and to both sides, as
    /// its [`TeleportView`](crate::TeleportView) is.
    Teleported {
        /// The seat whose hero channelled it.
        slot: SlotId,
        /// Whether the hero arrived. False when the channel was broken off or
        /// found no ground to land on.
        landed: bool,
    },
    /// Something thrown at a spot came down there and went off, as a Blood
    /// Grenade bursts. Told to the sides that see the spot, or to the
    /// thrower's when none does.
    Landed {
        /// What was thrown.
        inflictor: Inflictor,
        /// Who threw it, told as a blow's dealer is; may name a unit that has
        /// since died.
        caster: Option<EntityId>,
        /// Where it went off.
        at: Vec2,
    },
}

impl EventKind {
    /// Calls `visit` on every entity handle the event carries.
    pub fn visit_ids(&mut self, visit: &mut dyn FnMut(&mut EntityId)) {
        match self {
            EventKind::Damaged { source, target, .. }
            | EventKind::Missed { source, target }
            | EventKind::Healed { source, target, .. } => {
                source.iter_mut().for_each(&mut *visit);
                visit(target);
            }
            EventKind::Died { unit, killer, .. } => {
                visit(unit);
                killer.iter_mut().for_each(visit);
            }
            EventKind::Landed { caster, .. } => caster.iter_mut().for_each(visit),
            EventKind::AbilityCast { caster: unit, .. }
            | EventKind::LevelUp { unit, .. }
            | EventKind::StructureDestroyed { unit, .. } => visit(unit),
            EventKind::ItemBought { .. } | EventKind::Teleported { .. } => {}
        }
    }
}
