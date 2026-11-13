use bota_proto::{Fixed, Inflictor, Vec2};

use crate::engine::Entity;
use crate::engine::{Fnv, StateHash, state_hash};

/// A chain of damage leaping from one enemy to the next, a beat apart.
///
/// It carries a [`Transform`] for the spot its last leap landed and a team of
/// its own; whoever cast it may be gone by the time it has run out.
///
/// [`Transform`]: crate::game::Transform
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    /// Who cast it. The handle may outlive the body.
    pub source: Entity,
    /// The spell it is.
    pub inflictor: Inflictor,
    /// Where the last leap landed, kept for when its target has fallen.
    pub at: Vec2,
    /// Whom it has struck already, never struck twice.
    pub struck: Vec<Entity>,
    /// Leaps it may still make.
    pub leaps_left: u8,
    /// Damage the next leap deals.
    pub damage: i32,
    /// Percent of the damage the leap after keeps.
    pub keep_pct: i32,
    /// How far a leap reaches from the last spot.
    pub radius: Fixed,
    /// Ticks between two leaps.
    pub delay: u32,
    /// Ticks before the next leap.
    pub wait: u32,
    /// Whether it leaps on once its source is dead.
    pub life: ChainLife,
}

/// How long a chain leaps with regard to its source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChainLife {
    /// It leaps on whatever becomes of its source: a neutral's Chain Lightning.
    #[default]
    Outlives,
    /// It stops as its source dies: an item's ("If the wielder dies, all their currently ongoing
    /// Chain Lightning instances stop immediately", Liquipedia, Maelstrom).
    EndsWithSource,
}

#[cfg(test)]
impl crate::engine::Specimen for ChainLife {
    fn low() -> Self {
        ChainLife::Outlives
    }

    fn high() -> Self {
        ChainLife::EndsWithSource
    }
}

/// A chain that outlives its source hashes as one before the field was.
impl StateHash for ChainLife {
    fn state_hash(&self, fnv: &mut Fnv) {
        match self {
            ChainLife::Outlives => {}
            ChainLife::EndsWithSource => fnv.u8(1),
        }
    }
}

state_hash!(Chain {
    source,
    inflictor,
    at,
    struck,
    leaps_left,
    damage,
    keep_pct,
    radius,
    delay,
    wait,
    life
});
