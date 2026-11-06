//! Conditions used by simulation rules and exposed in unit views.

use serde::{Deserialize, Serialize};

/// Conditions currently affecting a unit, as a bit set of the associated
/// constants.
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct StatusFlags {
    /// The raw bits.
    pub bits: u16,
}

impl StatusFlags {
    /// Cannot act, move or turn.
    pub const STUNNED: u16 = 1 << 0;
    /// Movement speed is reduced.
    pub const SLOWED: u16 = 1 << 4;
    /// Losing health over time.
    pub const DOT: u16 = 1 << 5;
    /// Seen by the other team only through true sight.
    pub const INVISIBLE: u16 = 1 << 6;
    /// Immune to magical damage and most disables.
    pub const MAGIC_IMMUNE: u16 = 1 << 7;
    /// Cannot be attacked or damaged at all.
    pub const INVULNERABLE: u16 = 1 << 9;
    /// Performing a channelled ability or item action.
    pub const CHANNELLING: u16 = 1 << 10;
    /// Running from whoever put the fear on; cannot attack or cast.
    pub const FEARED: u16 = 1 << 11;
    /// In the cast point of an ability or an item: a new order to the body
    /// breaks the cast off before anything is spent.
    pub const CASTING: u16 = 1 << 12;
}
