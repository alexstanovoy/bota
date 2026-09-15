//! Grid pathfinding around structures.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bota_proto::{Fixed, Vec2};

use crate::game::{PassGrid, rules};

/// Whether a body of a collision size can walk the straight segment: it
/// crosses only walkable cells and keeps the body clear of every post.
///
/// Cells are sampled every half-cell along the line, which cannot skip over
/// a cell at that spacing; posts are met as the circles they are.
pub fn grid_los(grid: &PassGrid, from: Vec2, to: Vec2, room: Fixed) -> bool {
    let dx = i64::from(to.x.raw) - i64::from(from.x.raw);
    let dy = i64::from(to.y.raw) - i64::from(from.y.raw);
    let sample = i64::from(rules::GRID_CELL_SIZE) << 15; // half a cell, raw
    let len = dx.abs().max(dy.abs());
    let steps = (len / sample + 1).max(1);
    for i in 0..=steps {
        let p = Vec2 {
            x: bota_proto::Fixed {
                raw: (i64::from(from.x.raw) + dx * i / steps) as i32,
            },
            y: bota_proto::Fixed {
                raw: (i64::from(from.y.raw) + dy * i / steps) as i32,
            },
        };
        if !grid.walkable(p) {
            return false;
        }
    }
    grid.clear_of_posts(from, to, room)
}

const CELLS: usize = rules::GRID_CELLS;
const STRAIGHT: u32 = 64;
const DIAGONAL: u32 = 90;

fn heuristic(a: (usize, usize), b: (usize, usize)) -> u32 {
    let dx = a.0.abs_diff(b.0) as u32;
    let dy = a.1.abs_diff(b.1) as u32;
    STRAIGHT * dx.max(dy) + (DIAGONAL - STRAIGHT) * dx.min(dy)
}

/// The spot a body of a collision size may actually stand on beside a point
/// that may sit inside a building's footprint: the point itself when there
/// is room for it, else the centre of the first cell with room on the way
/// out of the footprint towards `toward`.
///
/// Falls back on the nearest cell with room in any direction when that way
/// out is shut too.
pub fn open_beside(grid: &PassGrid, at: Vec2, toward: Vec2, room: Fixed) -> Vec2 {
    if grid.walkable_for(at, room) {
        return at;
    }
    cell_beside(grid, at, toward, room)
        .or_else(|| PassGrid::cell_of(at).and_then(|cell| routable_cell(grid, cell, room)))
        .map_or(at, PassGrid::cell_center)
}

/// The first cell with room for a body on the way from a point towards
/// another, the point's own cell included, within [`OPEN_SEARCH_CELLS`].
/// None with the whole way shut, or the two points one.
fn cell_beside(grid: &PassGrid, at: Vec2, toward: Vec2, room: Fixed) -> Option<(usize, usize)> {
    let dx = i64::from(toward.x.raw) - i64::from(at.x.raw);
    let dy = i64::from(toward.y.raw) - i64::from(at.y.raw);
    let len = dx.abs().max(dy.abs());
    if len == 0 {
        return None;
    }
    let sample = i64::from(rules::GRID_CELL_SIZE) << 15; // half a cell, raw
    let steps = i64::from(OPEN_SEARCH_CELLS) * 2;
    for step in 0..=steps {
        let p = Vec2 {
            x: bota_proto::Fixed {
                raw: (i64::from(at.x.raw) + dx * sample * step / len) as i32,
            },
            y: bota_proto::Fixed {
                raw: (i64::from(at.y.raw) + dy * sample * step / len) as i32,
            },
        };
        if let Some(cell) = PassGrid::cell_of(p)
            && grid.fits(cell.0, cell.1, room)
        {
            return Some(cell);
        }
    }
    None
}

/// How many cells out a cell with room is looked for: across the widest
/// footprint on any map and the widest body, and one more.
const OPEN_SEARCH_CELLS: i32 =
    (rules::DIRE_ANCIENT_COLLISION + rules::STEER_MARGIN + rules::WIDEST_MARCHER)
        / rules::GRID_CELL_SIZE
        + 2;

/// The cell to route a body to for a goal: the goal cell itself, or the
/// cell with room nearest to it when there is none there.
///
/// Ties break on the lower row, then the lower column. None when nothing
/// within [`OPEN_SEARCH_CELLS`] has room.
fn routable_cell(grid: &PassGrid, cell: (usize, usize), room: Fixed) -> Option<(usize, usize)> {
    if grid.fits(cell.0, cell.1, room) {
        return Some(cell);
    }
    let mut best: Option<(i32, (usize, usize))> = None;
    for ring in 1..=OPEN_SEARCH_CELLS {
        if best.is_some_and(|(had, _)| ring * ring > had) {
            break;
        }
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let nx = cell.0 as i32 + dx;
                let ny = cell.1 as i32 + dy;
                if nx < 0 || ny < 0 || nx as usize >= CELLS || ny as usize >= CELLS {
                    continue;
                }
                if !grid.fits(nx as usize, ny as usize, room) {
                    continue;
                }
                let apart = dx * dx + dy * dy;
                if best.is_none_or(|(had, _)| apart < had) {
                    best = Some((apart, (nx as usize, ny as usize)));
                }
            }
        }
    }
    best.map(|(_, found)| found)
}

const NEIGHBOURS: [(i32, i32); 8] = [
    (1, 0),
    (0, 1),
    (-1, 0),
    (0, -1),
    (1, 1),
    (-1, 1),
    (-1, -1),
    (1, -1),
];

/// A* over the passability grid, returning the corners of the walk, the
/// last of them where the walk ends: at `to` when it can be stood on and
/// reached, else at the open spot nearest to it that can, on the walker's
/// own side of whatever shuts it.
///
/// Empty when the walk ends in the cell the walker stands in. Diagonal
/// steps never cut a blocked corner. Ties break on cell index, so the route
/// is the same on every platform.
pub fn find_path(grid: &PassGrid, from: Vec2, to: Vec2, room: Fixed) -> Vec<Vec2> {
    let (Some(start), Some(asked)) = (PassGrid::cell_of(from), PassGrid::cell_of(to)) else {
        return Vec::new();
    };
    let (Some(start), Some(goal)) = (
        routable_cell(grid, start, room),
        cell_beside(grid, to, from, room).or_else(|| routable_cell(grid, asked, room)),
    ) else {
        return Vec::new();
    };
    if start == goal {
        return Vec::new();
    }
    let idx = |c: (usize, usize)| c.1 * CELLS + c.0;
    let mut best = vec![u32::MAX; CELLS * CELLS];
    let mut parent = vec![u32::MAX; CELLS * CELLS];
    let mut heap = BinaryHeap::new();
    best[idx(start)] = 0;
    heap.push(Reverse((heuristic(start, goal), idx(start) as u32)));
    // The cell got to that lies nearest the goal, for when no way leads
    // there.
    let mut nearest = (heuristic(start, goal), idx(start));
    while let Some(Reverse((_, at))) = heap.pop() {
        let at = at as usize;
        let cell = (at % CELLS, at / CELLS);
        if cell == goal {
            break;
        }
        let left = heuristic(cell, goal);
        if left < nearest.0 {
            nearest = (left, at);
        }
        let g = best[at];
        for (i, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
            let nx = cell.0 as i32 + dx;
            let ny = cell.1 as i32 + dy;
            if nx < 0 || ny < 0 || nx as usize >= CELLS || ny as usize >= CELLS {
                continue;
            }
            let next = (nx as usize, ny as usize);
            if !grid.fits(next.0, next.1, room) {
                continue;
            }
            let diagonal = i >= 4;
            if diagonal && (!grid.fits(next.0, cell.1, room) || !grid.fits(cell.0, next.1, room)) {
                continue; // no cutting a blocked corner
            }
            let cost = g + if diagonal { DIAGONAL } else { STRAIGHT };
            let ni = idx(next);
            if cost < best[ni] {
                best[ni] = cost;
                parent[ni] = at as u32;
                heap.push(Reverse((cost + heuristic(next, goal), ni as u32)));
            }
        }
    }
    let end = if parent[idx(goal)] == u32::MAX {
        nearest.1
    } else {
        idx(goal)
    };
    if end == idx(start) {
        return Vec::new();
    }
    let goal = (end % CELLS, end / CELLS);
    // Walk the parents back, then keep only the corners.
    let mut cells = vec![goal];
    let mut at = end;
    while at != idx(start) {
        at = parent[at] as usize;
        cells.push((at % CELLS, at / CELLS));
    }
    cells.reverse();
    let mut corners: Vec<(usize, usize)> = Vec::new();
    for i in 1..cells.len() {
        let dir = (
            cells[i].0 as i32 - cells[i - 1].0 as i32,
            cells[i].1 as i32 - cells[i - 1].1 as i32,
        );
        let prev_dir = if i >= 2 {
            Some((
                cells[i - 1].0 as i32 - cells[i - 2].0 as i32,
                cells[i - 1].1 as i32 - cells[i - 2].1 as i32,
            ))
        } else {
            None
        };
        if prev_dir.is_some_and(|p| p != dir)
            && let Some(&last) = cells.get(i - 1)
        {
            corners.push(last);
        }
    }
    corners.push(goal);
    let mut spots: Vec<Vec2> = corners.iter().map(|&c| PassGrid::cell_center(c)).collect();
    if goal == asked {
        *spots.last_mut().expect("the end is kept") = to;
    }
    pull_string(grid, from, spots, room)
}

/// The corners a walk keeps: from where it stands and from each corner kept,
/// the walk goes straight to the farthest later corner the grid line
/// reaches, so a route that stepped round a footprint cell by cell rounds it
/// in a few straight legs. The last corner is always kept.
fn pull_string(grid: &PassGrid, from: Vec2, corners: Vec<Vec2>, room: Fixed) -> Vec<Vec2> {
    let mut kept = Vec::with_capacity(corners.len());
    let mut anchor = from;
    let mut at = 0;
    while at < corners.len() {
        let mut far = at;
        for (later, &corner) in corners.iter().enumerate().skip(at + 1) {
            if grid_los(grid, anchor, corner, room) {
                far = later;
            }
        }
        anchor = corners[far];
        kept.push(anchor);
        at = far + 1;
    }
    kept
}
