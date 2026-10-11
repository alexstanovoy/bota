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

/// One viewer lighting the fog: position, vision radius, ground tier.
struct Viewer {
    x: f32,
    y: f32,
    radius: f32,
    tier: u8,
}

/// Marks in `lit`, one entry per terrain cell row-major from the south-west,
/// every cell a unit of `team` sees: within its vision radius, on ground no
/// higher than its own, with no opaque cell or higher ground across the line
/// to it. Leaves `lit` untouched when the terrain is unknown or `lit` is not
/// one entry per cell.
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
    /// One bit a cell: whether it blocks sight.
    pub opaque: Vec<u64>,
}

impl FogGround {
    /// The app's ground.
    pub fn of(app: &App) -> FogGround {
        FogGround {
            cells: app.terrain_cells,
            terrain: app.terrain.clone(),
            opaque: app.opaque.clone(),
        }
    }
}

/// [`light`] over a [`FogGround`] taken from the app.
pub fn light_on(ground: &FogGround, view: &WorldView, team: Team, lit: &mut [bool]) {
    let n = ground.cells;
    if n == 0 || ground.terrain.len() != n * n || lit.len() != n * n {
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

/// Whether one viewer sees one cell.
fn sees(app: &FogGround, v: &Viewer, cx: usize, cy: usize) -> bool {
    let n = app.cells;
    let (px, py) = (cx as f32 * CELL + CELL / 2.0, cy as f32 * CELL + CELL / 2.0);
    let (dx, dy) = (px - v.x, py - v.y);
    if dx * dx + dy * dy > v.radius * v.radius || app.terrain[cy * n + cx] & 0x1f > v.tier {
        return false;
    }
    let steps = ((dx.abs().max(dy.abs()) / (CELL / 2.0)) as i32 + 1).max(1);
    let from = ((v.x / CELL) as usize, (v.y / CELL) as usize);
    for i in 1..steps {
        let sx = v.x + dx * i as f32 / steps as f32;
        let sy = v.y + dy * i as f32 / steps as f32;
        let sc = ((sx / CELL) as usize, (sy / CELL) as usize);
        if sc == from || sc == (cx, cy) {
            continue;
        }
        if sc.0 >= n || sc.1 >= n {
            return false;
        }
        let idx = sc.1 * n + sc.0;
        let opaque = app
            .opaque
            .get(idx / 64)
            .is_some_and(|w| w & (1 << (idx % 64)) != 0);
        if opaque || app.terrain[idx] & 0x1f > v.tier {
            return false;
        }
    }
    true
}
