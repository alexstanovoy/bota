//! Runes: the ones lying on the ground and the one a Bottle holds.

use bota_proto::RuneKind;

use crate::engine::state_hash;

/// A rune lying on the ground. The entity has a place and a row of sight,
/// as a ground item has, and no team, no health and no hull.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rune {
    /// Which rune it is.
    pub kind: RuneKind,
    /// The spot of the map it lies at, as an index into
    /// [`bota_core::MapDef::rune_spots`].
    pub spot: u8,
    /// The gold a Bounty Rune pays every hero of the side that takes it,
    /// fixed when it appeared. Zero for any other.
    pub gold: i32,
}

/// The rune a Bottle keeps (Dota's Store Rune).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeldRune {
    /// Which rune it is.
    pub kind: RuneKind,
    /// Ticks before it takes effect by itself.
    pub ticks_left: u32,
    /// The gold a bottled Bounty Rune pays: what it was worth when it
    /// appeared, not when it was bottled [LIQ Runes]. Zero for any other.
    pub gold: i32,
    /// Whether the Bottle was full as the rune was stored, which decides how
    /// much a Bounty or Water Rune refills once it takes effect.
    pub was_full: bool,
}

state_hash!(RuneKind as u8: RuneKind::Water, RuneKind::Bounty);
state_hash!(Rune { kind, spot, gold });
state_hash!(HeldRune {
    kind,
    ticks_left,
    gold,
    was_full
});
