//! bota's frame of the map: Dota's, shifted by half the map so the map spans `0..MAP_SIZE` on both
//! axes, cut into square cells. The numbers are `bota_core::rules`' `MAP_SIZE`, `GRID_CELLS` and
//! `GRID_CELL_SIZE`; the committed tables fail to build against a differing layout.

/// The map's side, in world units.
pub const MAP_SIZE: i32 = 18432;
/// What Dota's coordinates are shifted by.
pub const HALF: i32 = MAP_SIZE / 2;
/// A cell's side, in world units: Dota's navigation cell.
pub const CELL: i32 = 64;
/// Cells per side.
pub const CELLS: usize = (MAP_SIZE / CELL) as usize;

/// A point of Dota's frame in bota's.
pub trait Shifted {
    /// The point moved by half the map on both horizontal axes.
    fn shifted(self) -> Self;
}

impl Shifted for [f64; 3] {
    fn shifted(self) -> Self {
        [
            self[0] + f64::from(HALF),
            self[1] + f64::from(HALF),
            self[2],
        ]
    }
}

impl Shifted for [f64; 2] {
    fn shifted(self) -> Self {
        [self[0] + f64::from(HALF), self[1] + f64::from(HALF)]
    }
}

/// `value` rounded to the nearest whole number, a half to the even one: how every landmark of
/// the committed map was rounded.
pub fn whole(value: f64) -> i64 {
    value.round_ties_even() as i64
}

/// Whether a point of bota's frame lies on the map.
pub fn on_map(x: i64, y: i64) -> bool {
    (0..i64::from(MAP_SIZE)).contains(&x) && (0..i64::from(MAP_SIZE)).contains(&y)
}
