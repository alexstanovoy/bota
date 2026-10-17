//! Sight lines over the 64-unit cells, as Dota's own fog grid has them.
//!
//! A cell stops a viewer's line when its ground is higher than the viewer's,
//! when a fog blocker wall crosses it, or when a standing tree covers it and
//! the viewer stands no more than [`rules::TREE_SIGHT_TIERS_ABOVE`] above the
//! tree. Everything here is computed from what a match publishes: the
//! terrain, the fog blocker cells and the standing trees.

use bota_proto::{Fixed, Vec2};

use crate::{CellGrid, rules};

/// What stops a ground sight line, cell by cell.
///
/// Each cell holds the lowest tier a viewer must stand on for a line to
/// cross it: its ground's own tier, two above a tree covering it, or never
/// for a wall. Laid afresh whenever the forest changes, read on every line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SightBlock {
    /// One byte per cell, row-major from the south-west corner.
    floor: Vec<u8>,
}

/// The floor of a cell a fog blocker wall crosses: no viewer looks past it.
const WALL_FLOOR: u8 = u8::MAX;

impl SightBlock {
    /// The ground alone, with no wall and no tree: `tier_at` is the
    /// elevation tier under a position.
    pub fn of_ground(tier_at: &impl Fn(Vec2) -> u8) -> SightBlock {
        let mut floor = vec![0; rules::GRID_CELLS * rules::GRID_CELLS];
        for cy in 0..rules::GRID_CELLS {
            for cx in 0..rules::GRID_CELLS {
                floor[cy * rules::GRID_CELLS + cx] = tier_at(CellGrid::cell_center((cx, cy)));
            }
        }
        SightBlock { floor }
    }

    /// The ground, the closed cells of `walls`, the map's own trees in
    /// `rooted` and the trees items put up in `planted`.
    ///
    /// A map tree covers every cell whose centre lies within
    /// `TREE_SIGHT_RADIUS_SQUARED` of it: the 2x2 cells of the square it
    /// stands on, so a row of trees two cells apart is a closed wall. A
    /// planted tree, smaller, covers the one cell it stands in.
    pub fn lay(
        tier_at: &impl Fn(Vec2) -> u8,
        walls: &CellGrid,
        rooted: impl IntoIterator<Item = Vec2>,
        planted: impl IntoIterator<Item = Vec2>,
    ) -> SightBlock {
        let mut block = SightBlock::of_ground(tier_at);
        for at in rooted {
            let floor = tier_at(at) + rules::TREE_SIGHT_TIERS_ABOVE + 1;
            block.cover_disc(at, floor);
        }
        for at in planted {
            if let Some(cell) = CellGrid::cell_of(at) {
                block.raise(cell, tier_at(at) + rules::TREE_SIGHT_TIERS_ABOVE + 1);
            }
        }
        for cy in 0..rules::GRID_CELLS {
            for cx in 0..rules::GRID_CELLS {
                if !walls.cell_open(cx, cy) {
                    block.raise((cx, cy), WALL_FLOOR);
                }
            }
        }
        block
    }

    /// Raises to `floor` every cell whose centre lies within the tree
    /// blocker radius of `at`.
    fn cover_disc(&mut self, at: Vec2, floor: u8) {
        let reach = Fixed::from_int(rules::GRID_CELL_SIZE * 2);
        let low = CellGrid::cell_of(Vec2 {
            x: (at.x - reach).max(Fixed::ZERO),
            y: (at.y - reach).max(Fixed::ZERO),
        });
        let Some((x0, y0)) = low else {
            return;
        };
        let x1 = (x0 + 4).min(rules::GRID_CELLS - 1);
        let y1 = (y0 + 4).min(rules::GRID_CELLS - 1);
        let unit = i64::from(Fixed::ONE.raw);
        let radius_squared = rules::TREE_SIGHT_RADIUS_SQUARED * unit * unit;
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if CellGrid::cell_center((cx, cy)).distance_squared(at) <= radius_squared {
                    self.raise((cx, cy), floor);
                }
            }
        }
    }

    /// Lifts a cell's floor to at least `floor`.
    fn raise(&mut self, cell: (usize, usize), floor: u8) {
        let slot = &mut self.floor[cell.1 * rules::GRID_CELLS + cell.0];
        *slot = (*slot).max(floor);
    }

    /// Whether a cell stops the line of a viewer standing on `viewer_tier`.
    #[inline]
    pub fn stops(&self, cell: (usize, usize), viewer_tier: u8) -> bool {
        self.floor[cell.1 * rules::GRID_CELLS + cell.0] > viewer_tier
    }

    /// Whether the ground sight line from a viewer on `viewer_tier` to a
    /// point crosses no cell that stops it.
    ///
    /// The line visits every cell it passes through, and slips between two
    /// cells that only touch at a corner it runs through exactly. The
    /// viewer's own cell and the target's never stop it, and neither does
    /// anything around a target within [`rules::NEAR_SIGHT_CELLS_SQUARED`]
    /// of the viewer's cell: Dota shows a unit its closest tiles even from
    /// inside a forest.
    #[inline]
    pub fn clear(&self, from: Vec2, viewer_tier: u8, to: Vec2) -> bool {
        let (Some(start), Some(end)) = (CellGrid::cell_of(from), CellGrid::cell_of(to)) else {
            return false;
        };
        let cells_x = end.0.abs_diff(start.0) as i64;
        let cells_y = end.1.abs_diff(start.1) as i64;
        if cells_x * cells_x + cells_y * cells_y <= rules::NEAR_SIGHT_CELLS_SQUARED {
            return true;
        }
        let x = Axis::of(from.x, to.x, start.0);
        let y = Axis::of(from.y, to.y, start.1);
        // A border is crossed at `left / run` of the way along the line;
        // both axes' times share the denominator `run_x * run_y`, with a
        // run of zero, never crossing, counted as one.
        let (scale_x, scale_y) = (y.run.max(1), x.run.max(1));
        let mut time_x = x.first_time(scale_x);
        let mut time_y = y.first_time(scale_y);
        let (step_x, step_y) = (x.cell * scale_x, y.cell * scale_y);
        let end_time = scale_x * scale_y;
        let mut cell = (start.0 as i64, start.1 as i64);
        for _ in 0..cells_x + cells_y {
            let next = time_x.min(time_y);
            if next >= end_time {
                break;
            }
            if time_x == next {
                cell.0 += x.step;
                time_x += step_x;
            }
            if time_y == next {
                cell.1 += y.step;
                time_y += step_y;
            }
            let at = (cell.0 as usize, cell.1 as usize);
            if at == end {
                break;
            }
            if self.stops(at, viewer_tier) {
                return false;
            }
        }
        true
    }
}

/// One axis of a sight line walked cell by cell, in raw fixed units.
struct Axis {
    /// The cell index change at each border, `-1` or `1`.
    step: i64,
    /// How far the line runs along this axis in all.
    run: i64,
    /// How far from its start the line meets the first border ahead.
    left: i64,
    /// How far apart two borders are: one cell.
    cell: i64,
}

impl Axis {
    /// The axis of a line from `from` to `to` that starts in cell `start`.
    #[inline]
    fn of(from: Fixed, to: Fixed, start: usize) -> Axis {
        let cell = i64::from(rules::GRID_CELL_SIZE) * i64::from(Fixed::ONE.raw);
        let low = start as i64 * cell;
        let run = i64::from(to.raw) - i64::from(from.raw);
        let left = if run < 0 {
            i64::from(from.raw) - low
        } else {
            low + cell - i64::from(from.raw)
        };
        let step = if run < 0 { -1 } else { 1 };
        Axis {
            step,
            run: run.abs(),
            left,
            cell,
        }
    }

    /// When the first border is crossed, over the shared denominator; never
    /// for an axis the line does not run along.
    #[inline]
    fn first_time(&self, scale: i64) -> i64 {
        if self.run == 0 {
            i64::MAX
        } else {
            self.left * scale
        }
    }
}
