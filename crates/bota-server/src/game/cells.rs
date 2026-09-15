//! One bit per terrain cell.

use bota_proto::Vec2;

use crate::game::rules;

/// One bit per cell of the map's terrain layout, row-major: `true` is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellGrid {
    /// The bits, 64 cells to a word.
    bits: Vec<u64>,
}

impl CellGrid {
    /// A grid with every cell open.
    pub fn open() -> CellGrid {
        CellGrid {
            bits: vec![u64::MAX; rules::GRID_CELLS * rules::GRID_CELLS / 64],
        }
    }

    /// The cell a position falls into, if it is on the map.
    pub fn cell_of(pos: Vec2) -> Option<(usize, usize)> {
        if pos.x.raw < 0 || pos.y.raw < 0 {
            return None;
        }
        let cx = pos.x.to_int() / rules::GRID_CELL_SIZE;
        let cy = pos.y.to_int() / rules::GRID_CELL_SIZE;
        if cx >= rules::GRID_CELLS as i32 || cy >= rules::GRID_CELLS as i32 {
            return None;
        }
        Some((cx as usize, cy as usize))
    }

    /// The centre of a cell.
    pub fn cell_center(cell: (usize, usize)) -> Vec2 {
        Vec2::from_ints(
            cell.0 as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
            cell.1 as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
        )
    }

    /// Whether a cell is open.
    pub fn cell_open(&self, cx: usize, cy: usize) -> bool {
        let idx = cy * rules::GRID_CELLS + cx;
        self.bits[idx / 64] & (1 << (idx % 64)) != 0
    }

    /// Whether a position is on the map and in an open cell.
    pub fn open_at(&self, pos: Vec2) -> bool {
        match CellGrid::cell_of(pos) {
            None => false,
            Some((cx, cy)) => self.cell_open(cx, cy),
        }
    }

    /// Closes one cell.
    pub fn close_cell(&mut self, cx: usize, cy: usize) {
        let idx = cy * rules::GRID_CELLS + cx;
        self.bits[idx / 64] &= !(1 << (idx % 64));
    }
}
