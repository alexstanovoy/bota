//! Fog of war.
//!
//! Vision is a radius with sight lines over the grid: a point is visible
//! to a team when any of its units with a vision radius stands close
//! enough, on ground at least as high as the point, with no opaque cell in
//! between. A cell is opaque to a viewer when its ground is higher than
//! the viewer's or a tree stands on it; buildings and water block nothing.
//! Nothing is cached; the world is small enough to ask directly.

use bota_proto::{Fixed, Vec2};

use crate::game::{PassGrid, rules};

pub static SCRATCH_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The cells that block sight lines: every standing tree and the map's own
/// fog blocker walls, which is what seals the Roshan pit against looks
/// through its entrance.
pub fn build_sight_block(map: &crate::game::MapDef) -> PassGrid {
    let mut grid = build_fow_walls(map);
    for pos in crate::game::tree_positions(map) {
        if let Some((cx, cy)) = PassGrid::cell_of(pos) {
            grid.close_cell(cx, cy);
        }
    }
    grid
}

/// The fog blocker walls alone, with no tree closed.
///
/// What a forest that changes over a match is built on: the walls never move,
/// so they are laid once and the standing trees closed over them.
pub fn build_fow_walls(map: &crate::game::MapDef) -> PassGrid {
    let mut grid = PassGrid::open();
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

/// Every closed cell of the sight blocker grid, for the wire.
pub fn sight_block_cells(map: &crate::game::MapDef) -> Vec<(u16, u16)> {
    let grid = build_sight_block(map);
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
fn close_segment(grid: &mut PassGrid, a: Vec2, b: Vec2) {
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
        if let Some((cx, cy)) = PassGrid::cell_of(p) {
            grid.close_cell(cx, cy);
        }
    }
}

/// Whether the sight line from a viewer to a point crosses an opaque cell:
/// ground above the viewer's tier, a standing tree or a blocker wall. The
/// viewer's own cell and the target's cell never block, so a viewer beside
/// a tree is not blinded by it and a treeline's own edge stays visible.
pub fn sight_clear(
    ground: &crate::game::Ground,
    tree_cover: &PassGrid,
    from: Vec2,
    viewer_tier: u8,
    to: Vec2,
) -> bool {
    let (Some(from_cell), Some(to_cell)) = (PassGrid::cell_of(from), PassGrid::cell_of(to)) else {
        return false;
    };
    let dx = i64::from(to.x.raw) - i64::from(from.x.raw);
    let dy = i64::from(to.y.raw) - i64::from(from.y.raw);
    let sample = i64::from(rules::GRID_CELL_SIZE) << 15; // half a cell, raw
    let len = dx.abs().max(dy.abs());
    let steps = (len / sample + 1).max(1);
    for i in 1..steps {
        let p = Vec2 {
            x: Fixed {
                raw: (i64::from(from.x.raw) + dx * i / steps) as i32,
            },
            y: Fixed {
                raw: (i64::from(from.y.raw) + dy * i / steps) as i32,
            },
        };
        let Some(cell) = PassGrid::cell_of(p) else {
            return false;
        };
        if cell == from_cell || cell == to_cell {
            continue;
        }
        if ground.tier(p) > viewer_tier || !tree_cover.cell_open(cell.0, cell.1) {
            return false;
        }
    }
    true
}
