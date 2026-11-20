//! The ground cell by cell: where a unit may stand (the navigation grid), how high the ground
//! is (the world's solid collision, in the map's height steps) and where the river's water lies.

use heatel::kv3::Value;
use heatel::physics::Mesh;

use crate::frame::{CELL, CELLS, HALF, Shifted};
use crate::install::Map;
use crate::overrides::CellOverride;

/// Bit of a cell's byte: the navigation grid calls it walkable.
pub const WALKABLE: u8 = 0x80;
/// Bit of a cell's byte: river water covers it.
pub const WATER: u8 = 0x40;
/// The bits of a cell's byte that hold its height step.
pub const TIER: u8 = 0x1f;

/// How far from a triangle, in world units, a cell's centre still counts as on it: the map's
/// vertices sit a thousandth of a unit off the 64-unit lattice the cell centres lie on, so a
/// centre on a shared edge would otherwise fall between the triangles on either side.
const ON_TRIANGLE: f64 = 0.01;

/// Every cell of the ground, row-major from the map's south-west corner: [`WALKABLE`],
/// [`WATER`] and the height step ([`TIER`]).
pub fn cells(map: &Map, overrides: &[CellOverride]) -> Result<Vec<u8>, String> {
    let (step, lowest) = height_steps(map)?;
    if map.nav.edge != CELL as f32 {
        return Err(format!(
            "navigation cells of {} units, not {CELL}",
            map.nav.edge
        ));
    }
    let attributes = &map.world.attributes;
    let is_water = |mesh: &Mesh| {
        attributes
            .get(mesh.attributes)
            .is_some_and(|a| a.interacts_as.iter().any(|layer| layer == "water"))
    };
    let ground = meshes(map, |mesh| !is_water(mesh))?;
    let water = meshes(map, is_water)?;
    let top = highest(&ground);
    let wet = highest(&water);
    let first = -(HALF / CELL);
    let nav = &map.nav;
    let mut out = vec![0u8; CELLS * CELLS];
    for (index, cell) in out.iter_mut().enumerate() {
        let (column, row) = (index % CELLS, index / CELLS);
        let (x, y) = (first + column as i32, first + row as i32);
        // A cell the navigation grid leaves out is no ground a unit stands on; one with ground
        // under it would be a map the grid does not cover.
        if nav.flags(x, y).is_none() && top[index].is_some() {
            return Err(format!(
                "cell ({column}, {row}) has ground but lies outside the navigation grid"
            ));
        }
        let walkable = nav.walkable(x, y);
        // Ground below the map's lowest step (pits at its edges) counts as step 0, as the first
        // table had it; no ground at all under the centre (the void past the edge) too.
        let steps = top[index].map_or(0.0, |z| ((z - lowest) / step + 0.5).floor().max(0.0));
        if steps > f64::from(TIER) {
            return Err(format!(
                "cell ({column}, {row}) stands {steps} steps up, more than a cell's {TIER}"
            ));
        }
        let tier = steps as u8;
        *cell = tier
            | if walkable { WALKABLE } else { 0 }
            | if wet[index].is_some() { WATER } else { 0 };
    }
    for o in overrides {
        let (column, row) = o.cell;
        let cell = out
            .get_mut(usize::from(row) * CELLS + usize::from(column))
            .filter(|_| usize::from(column) < CELLS)
            .ok_or(format!(
                "an override names cell ({column}, {row}), off the map"
            ))?;
        let made = (*cell & !o.bits) | (o.value & o.bits);
        if made == *cell {
            return Err(format!(
                "the override of cell ({column}, {row}) is what the map gives: remove it"
            ));
        }
        *cell = made;
    }
    Ok(out)
}

/// The cells as runs of equal bytes, in order, none longer than a row; with `by_row`, none
/// crosses a row's end either.
pub fn runs(cells: &[u8], by_row: bool) -> Vec<(u16, u8)> {
    let mut out: Vec<(u16, u8)> = Vec::new();
    for (index, &cell) in cells.iter().enumerate() {
        let row_starts = by_row && index % CELLS == 0;
        match out.last_mut() {
            Some((length, value))
                if *value == cell && usize::from(*length) < CELLS && !row_starts =>
            {
                *length += 1
            }
            _ => out.push((1, cell)),
        }
    }
    out
}

/// The map's height step and its lowest height, from its `worldspawn`
/// (`_dotatilegrid_stepheight`, `_dotatilegrid_minheight`).
fn height_steps(map: &Map) -> Result<(f64, f64), String> {
    let world = map
        .entities
        .iter()
        .find(|e| e.get("classname").and_then(Value::as_str) == Some("worldspawn"))
        .ok_or("no worldspawn")?;
    let number = |key: &str| {
        world
            .get(key)
            .and_then(Value::as_str)
            .and_then(|text| text.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .ok_or(format!("worldspawn has no {key}"))
    };
    let step = number("_dotatilegrid_stepheight")?;
    if step <= 0.0 {
        return Err(format!("height step {step}"));
    }
    Ok((step, number("_dotatilegrid_minheight")?))
}

fn meshes(map: &Map, wanted: impl Fn(&Mesh) -> bool) -> Result<Vec<&Mesh>, String> {
    let found: Vec<&Mesh> = map.world.meshes.iter().filter(|m| wanted(m)).collect();
    match found.is_empty() {
        true => Err("the world has no such collision mesh".into()),
        false => Ok(found),
    }
}

/// The highest point of the meshes over each cell's centre, absent where none lies under it.
fn highest(meshes: &[&Mesh]) -> Vec<Option<f64>> {
    let mut top = vec![None::<f64>; CELLS * CELLS];
    let centre = |index: i64| index as f64 * f64::from(CELL) + f64::from(CELL) / 2.0;
    for mesh in meshes {
        for triangle in &mesh.triangles {
            let [a, b, c] = triangle.map(|i| mesh.vertices[i as usize].map(f64::from).shifted());
            let Some(on) = Triangle::new(a, b, c) else {
                continue;
            };
            let cells = |low: f64, high: f64| {
                let first = ((low - ON_TRIANGLE - f64::from(CELL) / 2.0) / f64::from(CELL)).ceil();
                let last = ((high + ON_TRIANGLE - f64::from(CELL) / 2.0) / f64::from(CELL)).floor();
                (first.max(0.0) as i64)..=(last.min(CELLS as f64 - 1.0) as i64)
            };
            for row in cells(on.min[1], on.max[1]) {
                for column in cells(on.min[0], on.max[0]) {
                    let Some(z) = on.height(centre(column), centre(row)) else {
                        continue;
                    };
                    let cell = &mut top[row as usize * CELLS + column as usize];
                    *cell = Some(cell.map_or(z, |was| was.max(z)));
                }
            }
        }
    }
    top
}

/// A triangle seen from above, which a point within [`ON_TRIANGLE`] of counts as on.
struct Triangle {
    corners: [[f64; 3]; 3],
    /// Twice its signed area seen from above.
    area: f64,
    /// For each corner's weight, how low it may go and the point still count as on.
    margins: [f64; 3],
    min: [f64; 2],
    max: [f64; 2],
}

impl Triangle {
    /// None for a triangle that stands on edge, which seen from above has no inside.
    fn new(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<Triangle> {
        let area = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if area.abs() < 1e-9 {
            return None;
        }
        let side = |p: [f64; 3], q: [f64; 3]| (p[0] - q[0]).hypot(p[1] - q[1]);
        let margin = |length: f64| -ON_TRIANGLE * length / area.abs();
        Some(Triangle {
            corners: [a, b, c],
            area,
            margins: [margin(side(b, c)), margin(side(c, a)), margin(side(a, b))],
            min: [a[0].min(b[0]).min(c[0]), a[1].min(b[1]).min(c[1])],
            max: [a[0].max(b[0]).max(c[0]), a[1].max(b[1]).max(c[1])],
        })
    }

    /// The height of the triangle over `(x, y)`, held within its corners' heights, if the point
    /// counts as on it.
    fn height(&self, x: f64, y: f64) -> Option<f64> {
        let [a, b, c] = self.corners;
        let wa = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / self.area;
        let wb = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / self.area;
        let wc = 1.0 - wa - wb;
        if wa < self.margins[0] || wb < self.margins[1] || wc < self.margins[2] {
            return None;
        }
        let z = wa * a[2] + wb * b[2] + wc * c[2];
        let low = a[2].min(b[2]).min(c[2]);
        let high = a[2].max(b[2]).max(c[2]);
        Some(z.clamp(low, high))
    }
}
