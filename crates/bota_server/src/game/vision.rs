//! The fog blocker walls of a map, as the cells they close. What a sight
//! line crosses is `bota_core::SightBlock`.

use bota_core::{CellGrid, rules};
use bota_proto::{Fixed, Vec2};

/// The fog blocker walls of a map alone, with no tree and no ground.
pub fn build_fow_walls(map: &crate::game::MapDef) -> CellGrid {
    let mut grid = CellGrid::open();
    for wall in map.fow_blockers {
        for seg in wall.windows(2) {
            let a = Vec2::from_ints(i32::from(seg[0].0), i32::from(seg[0].1));
            let b = Vec2::from_ints(i32::from(seg[1].0), i32::from(seg[1].1));
            // A named group holds several separate walls; only close nodes
            // span a wall between them.
            if a.within(b, rules::units(rules::FOW_BLOCKER_SPAN)) {
                close_segment(&mut grid, a, b);
            }
        }
    }
    grid
}

/// Every cell a fog blocker wall of a map closes, for the wire.
pub fn fow_wall_cells(map: &crate::game::MapDef) -> Vec<(u16, u16)> {
    let grid = build_fow_walls(map);
    let mut out = Vec::new();
    for cy in 0..rules::GRID_CELLS {
        for cx in 0..rules::GRID_CELLS {
            if !grid.cell_open(cx, cy) {
                out.push((cx as u16, cy as u16));
            }
        }
    }
    out
}

/// Closes every cell a segment passes through, sampled every half-cell.
fn close_segment(grid: &mut CellGrid, a: Vec2, b: Vec2) {
    let dx = i64::from(b.x.raw) - i64::from(a.x.raw);
    let dy = i64::from(b.y.raw) - i64::from(a.y.raw);
    let sample = i64::from(rules::GRID_CELL_SIZE) << 15;
    let len = dx.abs().max(dy.abs());
    let steps = (len / sample + 1).max(1);
    for i in 0..=steps {
        let p = Vec2 {
            x: Fixed {
                raw: (i64::from(a.x.raw) + dx * i / steps) as i32,
            },
            y: Fixed {
                raw: (i64::from(a.y.raw) + dy * i / steps) as i32,
            },
        };
        if let Some((cx, cy)) = CellGrid::cell_of(p) {
            grid.close_cell(cx, cy);
        }
    }
}
