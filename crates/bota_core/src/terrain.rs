//! How one cell of a map's terrain table is read: the same bits on the wire
//! as in the server.

/// Bits of a terrain cell that hold its elevation tier.
const TIER_MASK: u8 = 0x1f;
/// The bit of a terrain cell that marks river water.
const WATER_BIT: u8 = 0x40;
/// The bit of a terrain cell that marks ground a body may stand on.
const WALKABLE_BIT: u8 = 0x80;

/// The elevation tier of a terrain cell.
#[inline]
pub const fn cell_tier(cell: u8) -> u8 {
    cell & TIER_MASK
}

/// Whether a terrain cell is river water.
#[inline]
pub const fn cell_is_water(cell: u8) -> bool {
    cell & WATER_BIT != 0
}

/// Whether the terrain itself allows standing on a terrain cell.
#[inline]
pub const fn cell_is_walkable(cell: u8) -> bool {
    cell & WALKABLE_BIT != 0
}
