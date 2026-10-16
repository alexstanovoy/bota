//! A feast of souls on its caster: how far it has run and whom it has fed
//! on.

use crate::engine::Entity;

/// Where a feast stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeastPhase {
    /// Hastening its caster and taking souls from the enemies near him.
    Feasting,
    /// Over: the souls taken from enemies that have died since are kept past
    /// the cap until it runs out.
    Lingering,
}

/// One enemy a feast has taken souls from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fed {
    /// Whom they were taken from. The handle may outlive the body.
    pub from: Entity,
    /// How many were taken.
    pub souls: u32,
}

/// A feast of souls on the one who cast it. Gone once it has run out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feast {
    /// Which level of it was cast, counted from zero.
    pub level: u8,
    /// Where it stands.
    pub phase: FeastPhase,
    /// Ticks before the phase ends, at most
    /// [`rules::FEAST_TICKS`](bota_core::rules::FEAST_TICKS) or
    /// [`rules::FEAST_LINGER_TICKS`](bota_core::rules::FEAST_LINGER_TICKS).
    pub ticks_left: u32,
    /// Every enemy fed on, each once, at most
    /// [`rules::FEAST_TARGETS`](bota_core::rules::FEAST_TARGETS) at its
    /// level. While lingering, only those that have died.
    pub fed: Vec<Fed>,
}

impl Feast {
    /// Souls taken from everyone still on the list.
    pub fn souls(&self) -> u32 {
        self.fed.iter().map(|fed| fed.souls).sum()
    }
}
