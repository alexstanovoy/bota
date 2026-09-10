//! An effect a unit hands out to everyone standing near it.

use crate::game::StatusKind;

/// Whom an aura reaches, among those of its own side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Everything of its own side.
    All,
    /// Heroes and nothing else.
    Heroes,
}

/// One effect handed out by standing near something.
///
/// It reaches its own side only. Standing in it puts the effect on afresh
/// every tick, so what `ticks` buys is how long it lingers after walking out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aura {
    /// Which effect it hands out, and how much of it.
    pub kind: StatusKind,
    /// How far it reaches, in world units.
    pub radius: i32,
    /// Which of its own side it reaches.
    pub reaches: Reach,
    /// Ticks the effect holds once handed out.
    pub ticks: u32,
}

/// Everything an entity hands out. Absent when it hands out nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Auras(pub &'static [Aura]);
