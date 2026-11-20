//! Every number of the generated map that is not the install's: the one place a hand correction
//! lives, each with why.

/// A correction of bits of one cell of a map's ground: the `bits` of the cell's byte take those of
/// `value`. The tool refuses one that changes nothing, so a correction the map has caught up with
/// is noticed and removed.
pub struct CellOverride {
    /// `(column, row)` from the map's south-west corner.
    pub cell: (u16, u16),
    pub bits: u8,
    pub value: u8,
}

const fn tier(column: u16, row: u16, step: u8) -> CellOverride {
    CellOverride {
        cell: (column, row),
        bits: crate::ground::TIER,
        value: step,
    }
}

/// Height steps of single cells of the Dota map that the table made in August 2026 holds and the
/// map's collision does not give: 32 cells of 82,944, each on a step's edge (a wall's foot, a
/// ramp's lip, the rim of a thin collision sheet), where the cell centre's height lies within a
/// few units of a half step or a wall's top a thousandth of a unit away. The tool that made that
/// table (not kept) did its own floating-point arithmetic and fell on the other side. Kept so that
/// generating the old map changes nothing; retired with the next change of the ground.
pub const DOTA_CELLS: &[CellOverride] = &[
    tier(201, 6, 2),
    tier(209, 6, 2),
    tier(213, 6, 2),
    tier(217, 6, 2),
    tier(221, 6, 2),
    tier(166, 11, 0),
    tier(94, 23, 1),
    tier(159, 57, 0),
    tier(166, 67, 1),
    tier(165, 68, 1),
    tier(201, 83, 0),
    tier(202, 84, 0),
    tier(66, 156, 1),
    tier(67, 157, 1),
    tier(68, 158, 1),
    tier(120, 197, 1),
    tier(121, 197, 1),
    tier(122, 198, 1),
    tier(120, 226, 0),
    tier(19, 230, 0),
    tier(121, 255, 0),
    tier(121, 256, 0),
    tier(121, 257, 0),
    tier(121, 258, 0),
    tier(121, 259, 0),
    tier(121, 260, 0),
    tier(185, 260, 1),
    tier(121, 267, 0),
    tier(121, 268, 0),
    tier(121, 269, 0),
    tier(121, 270, 0),
    tier(121, 271, 0),
];

/// The corner each side lane bends through between its two tier-one towers, in bota's frame. bota's
/// lanes are straight chords between landmarks, not Dota's own road (`path_corner` chains): one
/// bend a side lane, chosen by hand where its road turns.
pub const LANE_BENDS: [(&str, (i32, i32)); 2] =
    [("TOP_CORNER", (3050, 15150)), ("BOT_CORNER", (15400, 2900))];

/// Positions, in bota's frame, that the committed map has and the rule (Dota's position shifted by
/// half the map, rounded to the nearest unit, a half to the even one) does not give, by the map
/// editor id of the entity. The Watcher at (-3483.5, 34.38) was typed by hand in October 2026 with
/// its half rounded up; kept so that generating the old map changes nothing.
pub const POSITION_OVERRIDES: &[(&str, (i64, i64))] = &[("2699:28432", (5733, 9250))];

/// Cells of the hero demo map's ground that the table made in August 2026 holds and the map's
/// collision does not give: 12 whose centre lies exactly half a step (64 units) up, which that
/// table's tool rounded down (to the even step), and one at the river's edge it left dry and on the
/// ground below. Kept so that generating the old map changes nothing.
pub const DEMO_CELLS: &[CellOverride] = &[
    tier(134, 131, 0),
    tier(133, 132, 0),
    tier(132, 133, 0),
    tier(131, 134, 0),
    tier(130, 135, 0),
    tier(129, 136, 0),
    CellOverride {
        cell: (128, 137),
        bits: crate::ground::WATER | crate::ground::TIER,
        value: 0x00,
    },
    tier(127, 138, 0),
    tier(142, 143, 0),
    tier(141, 144, 0),
    tier(138, 147, 0),
    tier(137, 148, 0),
    tier(136, 149, 0),
];
