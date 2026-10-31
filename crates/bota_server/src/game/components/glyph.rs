//! A side's Glyph of Fortification.

use crate::engine::state_hash;

/// The first tier one tower of the side has fallen and refreshed the Glyph.
pub const GLYPH_TIER_ONE: u8 = 1;
/// The first tier two tower of the side has fallen and refreshed the Glyph.
pub const GLYPH_TIER_TWO: u8 = 2;
/// The first melee barracks of the side has fallen and refreshed the Glyph.
pub const GLYPH_MELEE_RAX: u8 = 4;

/// One side's Glyph of Fortification: what its single cooldown, its
/// protection and the refreshes it has had are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// The tick the side may activate it from.
    pub ready_at: u32,
    /// Until this tick its structures take no damage.
    pub structures_until: u32,
    /// The refreshes it has had, as the `GLYPH_*` bits: each building falls
    /// to refresh it once.
    pub refreshed: u8,
}

impl Glyph {
    /// A Glyph not yet used, ready from the first moment Dota allows on a
    /// match with schedule `schedule`.
    pub const fn fresh(schedule: bota_core::rules::Schedule) -> Glyph {
        Glyph {
            ready_at: schedule.glyph_first_use(),
            structures_until: 0,
            refreshed: 0,
        }
    }
}

state_hash!(Glyph {
    ready_at,
    structures_until,
    refreshed
});
