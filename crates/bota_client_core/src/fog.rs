//! The client's own fog: which terrain cells the units of a side see.
//!
//! The server decides what a side may know; this only shades the ground and,
//! in a fogless replay watched as one seat, hides what that seat would not
//! see.

use bota_proto::{Team, WorldView};

use crate::state::App;

/// World units of one terrain cell.
pub const CELL: f32 = 64.0;

/// The ground tier under a world position; zero off the map.
pub fn tier_at(app: &App, x: f32, y: f32) -> u8 {
    tier_in(app.terrain_cells, &app.terrain, x, y)
}

fn ground_tier(ground: &FogGround, x: f32, y: f32) -> u8 {
    tier_in(ground.cells, &ground.terrain, x, y)
}

fn tier_in(n: usize, terrain: &[u8], x: f32, y: f32) -> u8 {
    if n == 0 || x < 0.0 || y < 0.0 || terrain.len() != n * n {
        return 0;
    }
    let (cx, cy) = ((x / CELL) as usize, (y / CELL) as usize);
    if cx >= n || cy >= n {
        return 0;
    }
    terrain[cy * n + cx] & 0x1f
}

/// One viewer lighting the fog: position, vision radius, ground tier. The
/// sight lines are walked over the wire's own fixed-point position, as the
/// server walks them, so the picture matches the rule cell for cell.
struct Viewer {
    x: f32,
    y: f32,
    radius: f32,
    tier: u8,
    /// The position in raw fixed units.
    at: (i64, i64),
    /// The vision radius in raw fixed units.
    reach: i64,
}

/// Raw fixed units of one terrain cell.
const CELL_RAW: i64 = 64 << 16;

/// Marks in `lit`, one entry per terrain cell row-major from the south-west,
/// every cell a unit of `team` sees: within its vision radius, on ground no
/// higher than its own, near it or with no wall, high ground or tree tall
/// enough across the line to it. The forest is the app's latest view's.
/// Leaves `lit` untouched when the terrain is unknown or `lit` is not one
/// entry per cell.
pub fn light(app: &App, view: &WorldView, team: Team, lit: &mut [bool]) {
    light_on(&FogGround::of(app), view, team, lit);
}

/// What the fog is worked out over: the terrain cells and what in them
/// blocks sight, owned, to be worked on away from the [`App`] (a front end
/// lights the fog on another thread).
#[derive(Clone, Debug, Default)]
pub struct FogGround {
    /// Cells a side.
    pub cells: usize,
    /// Each cell's ground tier in its low five bits, row-major from the
    /// south-west.
    pub terrain: Vec<u8>,
    /// One byte a cell: the lowest tier a viewer must stand on for a sight
    /// line to cross it. The server's own rule, `SightBlock` in
    /// `bota_server`'s `game/vision.rs`, pictured here.
    pub floor: Vec<u8>,
}

/// Squared world units around a map tree within which a cell's centre is
/// shut to sight: the 2×2 cells of the square it stands on.
const TREE_SIGHT_RADIUS_SQUARED: f32 = 2.0 * CELL * CELL;
/// How much higher than its ground a tree raises the floor of the cells it
/// covers: it blocks viewers on its level and one level up.
const TREE_FLOOR_ABOVE: u8 = 2;
/// Squared cell distance within which a viewer sees whatever stands
/// between.
const NEAR_CELLS_SQUARED: i64 = 5;

impl FogGround {
    /// The app's ground with the fog blocker walls and the forest as the
    /// latest view has it: the map's trees less the felled ones, and the
    /// planted ones.
    pub fn of(app: &App) -> FogGround {
        let n = app.terrain_cells;
        let mut ground = FogGround {
            cells: n,
            terrain: app.terrain.clone(),
            floor: app.terrain.iter().map(|cell| cell & 0x1f).collect(),
        };
        if n == 0 || ground.floor.len() != n * n {
            return ground;
        }
        let (felled, planted) = app.view.as_ref().map_or((&[][..], &[][..]), |view| {
            (&view.felled_trees[..], &view.planted_trees[..])
        });
        for (index, &(x, y)) in app.trees.iter().enumerate() {
            if !felled.contains(&(index as u32)) {
                ground.cover_disc(x, y);
            }
        }
        for at in planted {
            let (x, y) = (at.x.to_f32(), at.y.to_f32());
            if let Some(idx) = ground.index(x, y) {
                let floor = ground_tier(&ground, x, y) + TREE_FLOOR_ABOVE;
                ground.floor[idx] = ground.floor[idx].max(floor);
            }
        }
        for (idx, floor) in ground.floor.iter_mut().enumerate() {
            if app
                .opaque
                .get(idx / 64)
                .is_some_and(|w| w & (1 << (idx % 64)) != 0)
            {
                *floor = u8::MAX;
            }
        }
        ground
    }

    /// The cell a world point falls in, if on the ground.
    fn index(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (cx, cy) = ((x / CELL) as usize, (y / CELL) as usize);
        (cx < self.cells && cy < self.cells).then_some(cy * self.cells + cx)
    }

    /// Raises the floor of every cell whose centre lies within a map tree's
    /// blocker radius.
    fn cover_disc(&mut self, x: f32, y: f32) {
        let n = self.cells;
        let floor = ground_tier(self, x, y) + TREE_FLOOR_ABOVE;
        let c0 = ((x / CELL).floor() as i64 - 2).max(0) as usize;
        let r0 = ((y / CELL).floor() as i64 - 2).max(0) as usize;
        for cy in r0..(r0 + 5).min(n) {
            for cx in c0..(c0 + 5).min(n) {
                let dx = cx as f32 * CELL + CELL / 2.0 - x;
                let dy = cy as f32 * CELL + CELL / 2.0 - y;
                if dx * dx + dy * dy <= TREE_SIGHT_RADIUS_SQUARED {
                    let slot = &mut self.floor[cy * n + cx];
                    *slot = (*slot).max(floor);
                }
            }
        }
    }
}

/// [`light`] over a [`FogGround`] taken from the app.
pub fn light_on(ground: &FogGround, view: &WorldView, team: Team, lit: &mut [bool]) {
    let n = ground.cells;
    if n == 0 || ground.terrain.len() != n * n || ground.floor.len() != n * n || lit.len() != n * n
    {
        return;
    }
    lit.fill(false);
    let viewers = view
        .units
        .iter()
        .filter(|u| u.team == team && u.vision_radius.to_f32() > 0.0)
        .map(|u| {
            let (x, y) = (u.pos.x.to_f32(), u.pos.y.to_f32());
            Viewer {
                x,
                y,
                radius: u.vision_radius.to_f32(),
                tier: ground_tier(ground, x, y),
                at: (i64::from(u.pos.x.raw), i64::from(u.pos.y.raw)),
                reach: i64::from(u.vision_radius.raw),
            }
        });
    for v in viewers {
        let c0 = ((v.x - v.radius) / CELL).floor().max(0.0) as usize;
        let c1 = (((v.x + v.radius) / CELL).ceil().max(0.0) as usize).min(n - 1);
        let r0 = ((v.y - v.radius) / CELL).floor().max(0.0) as usize;
        let r1 = (((v.y + v.radius) / CELL).ceil().max(0.0) as usize).min(n - 1);
        for cy in r0..=r1 {
            for cx in c0..=c1 {
                if !lit[cy * n + cx] && sees(ground, &v, cx, cy) {
                    lit[cy * n + cx] = true;
                }
            }
        }
    }
}

/// Whether one viewer sees one cell: within its radius, on ground no higher
/// than its own, near it or along a line through no cell that stops it.
fn sees(ground: &FogGround, v: &Viewer, cx: usize, cy: usize) -> bool {
    let n = ground.cells;
    let to = (
        cx as i64 * CELL_RAW + CELL_RAW / 2,
        cy as i64 * CELL_RAW + CELL_RAW / 2,
    );
    let (dx, dy) = (to.0 - v.at.0, to.1 - v.at.1);
    if dx * dx + dy * dy > v.reach * v.reach || ground.terrain[cy * n + cx] & 0x1f > v.tier {
        return false;
    }
    if v.at.0 < 0 || v.at.1 < 0 {
        return false;
    }
    let from = (v.at.0 / CELL_RAW, v.at.1 / CELL_RAW);
    let (cells_x, cells_y) = ((cx as i64 - from.0).abs(), (cy as i64 - from.1).abs());
    if cells_x * cells_x + cells_y * cells_y <= NEAR_CELLS_SQUARED {
        return true;
    }
    // Border crossing times over the shared denominator `run_x * run_y`, a
    // run of zero counted as one and never crossing.
    let axis = |at: i64, cell: i64, run: i64| {
        let left = if run < 0 {
            at - cell * CELL_RAW
        } else {
            (cell + 1) * CELL_RAW - at
        };
        (if run < 0 { -1 } else { 1 }, left, run.abs())
    };
    let (step_x, left_x, run_x) = axis(v.at.0, from.0, dx);
    let (step_y, left_y, run_y) = axis(v.at.1, from.1, dy);
    let (scale_x, scale_y) = (run_y.max(1), run_x.max(1));
    let mut time_x = if run_x == 0 {
        i64::MAX
    } else {
        left_x * scale_x
    };
    let mut time_y = if run_y == 0 {
        i64::MAX
    } else {
        left_y * scale_y
    };
    let end_time = scale_x * scale_y;
    let mut cell = from;
    for _ in 0..cells_x + cells_y {
        let next = time_x.min(time_y);
        if next >= end_time {
            break;
        }
        if time_x == next {
            cell.0 += step_x;
            time_x += CELL_RAW * scale_x;
        }
        if time_y == next {
            cell.1 += step_y;
            time_y += CELL_RAW * scale_y;
        }
        if cell == (cx as i64, cy as i64) {
            break;
        }
        if cell.0 < 0 || cell.1 < 0 || cell.0 as usize >= n || cell.1 as usize >= n {
            return false;
        }
        if ground.floor[cell.1 as usize * n + cell.0 as usize] > v.tier {
            return false;
        }
    }
    true
}
