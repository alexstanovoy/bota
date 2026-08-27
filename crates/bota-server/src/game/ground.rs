//! The ground: elevation tiers, water and terrain walkability.

use bota_proto::Vec2;

use crate::game::{TERRAIN_CELLS, rules};

/// The decoded terrain, one byte per passability cell.
///
/// Bit 7 is ground the map's own gridnav calls walkable, bit 6 is river
/// water, the low bits are the elevation tier in 128-unit steps.
#[derive(Clone, Debug)]
pub struct Ground {
    cells: Vec<u8>,
}

impl Ground {
    /// The ground a map stands on, decoded from its baked table.
    pub fn of(map: &crate::game::MapDef) -> Ground {
        let mut cells = Vec::with_capacity(TERRAIN_CELLS * TERRAIN_CELLS);
        for &(run, value) in map.terrain_rle {
            for _ in 0..run {
                cells.push(value);
            }
        }
        debug_assert_eq!(cells.len(), TERRAIN_CELLS * TERRAIN_CELLS);
        Ground { cells }
    }

    fn cell(&self, pos: Vec2) -> u8 {
        let cx = pos.x.to_int() / rules::GRID_CELL_SIZE;
        let cy = pos.y.to_int() / rules::GRID_CELL_SIZE;
        if cx < 0 || cy < 0 || cx as usize >= TERRAIN_CELLS || cy as usize >= TERRAIN_CELLS {
            return 0;
        }
        self.cells[cy as usize * TERRAIN_CELLS + cx as usize]
    }

    /// The elevation tier under a position. Higher is higher ground.
    pub fn tier(&self, pos: Vec2) -> u8 {
        self.cell(pos) & 0x1f
    }

    /// Whether a position lies in river water.
    pub fn water(&self, pos: Vec2) -> bool {
        self.cell(pos) & 0x40 != 0
    }

    /// Whether the terrain itself allows standing on a cell.
    pub fn cell_walkable(&self, cx: usize, cy: usize) -> bool {
        cx < TERRAIN_CELLS && cy < TERRAIN_CELLS && self.cells[cy * TERRAIN_CELLS + cx] & 0x80 != 0
    }

    /// The wire form of a map's terrain: run-length pairs.
    pub fn wire_rle(map: &crate::game::MapDef) -> Vec<(u16, u8)> {
        map.terrain_rle.to_vec()
    }
}
