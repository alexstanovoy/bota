//! A feast of souls on its caster: whom it has fed on. How long it runs, its
//! level and the souls it shows are its modifiers', `ModifierKind::Feasting`
//! while it runs and `ModifierKind::FeastLinger` once it is over.

use crate::engine::Entity;

/// Which of its two modifiers a feast is on, so that its end is acted on
/// when that modifier lifts, however it lifts.
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
    pub souls: u32,
}

/// A feast of souls on the one who cast it. Gone once it has run out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feast {
    pub phase: FeastPhase,
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
