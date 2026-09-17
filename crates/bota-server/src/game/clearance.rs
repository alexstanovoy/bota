//! The ground as a body meets it: closed terrain as squares, trees and
//! structures as circles, and at every node of the walking lattice how far
//! the nearest of them stands.

use std::sync::OnceLock;

use bota_proto::{Fixed, Vec2};

use crate::game::{
    CellGrid, isqrt64, point_box_distance_squared, rules, segment_box_distance_squared,
    segment_distance_squared,
};

const NODES: usize = rules::WALK_CELLS;
const NODE: i32 = rules::WALK_CELL_SIZE;
const CAP: i32 = rules::CLEARANCE_CAP;
const BUCKETS: usize = rules::BUCKETS;
/// Half the diagonal of a node, rounded up, in world units.
const HALF_DIAGONAL: i32 = 23;

/// The static obstacles of a map and the room a body has anywhere on it.
///
/// Terrain closes whole cells, met as squares. A tree and a structure are
/// circles of their collision size, met exactly. At every node of the
/// walking lattice the room is the distance from the node's centre to the
/// nearest of them, capped at [`rules::CLEARANCE_CAP`]; zero on closed
/// ground.
#[derive(Clone, Debug)]
pub struct Clearance {
    /// The terrain, one bit per cell.
    terrain: CellGrid,
    /// The room at every node with only the terrain standing.
    terrain_room: Vec<u8>,
    /// The room at every node with everything standing, row-major.
    room: Vec<u8>,
    /// Every circle, centre and radius, in sorted order.
    circles: Vec<(Vec2, Fixed)>,
    /// Where each bucket's circles begin in `bucket_items`, one more entry
    /// than there are buckets.
    bucket_start: Vec<u32>,
    /// Indexes into `circles`, grouped by the bucket their centre falls in.
    bucket_items: Vec<u32>,
    /// The widest circle's radius, in whole world units.
    widest: i32,
}

/// The base field of each map, with everything the map starts with
/// standing, laid once per process.
static BASES: [OnceLock<Clearance>; crate::game::MAPS.len()] =
    [const { OnceLock::new() }; crate::game::MAPS.len()];

impl Clearance {
    /// Open ground everywhere, with nothing standing on it.
    pub fn open() -> Clearance {
        Clearance {
            terrain: CellGrid::open(),
            terrain_room: vec![CAP as u8; NODES * NODES],
            room: vec![CAP as u8; NODES * NODES],
            circles: Vec::new(),
            bucket_start: vec![0; BUCKETS * BUCKETS + 1],
            bucket_items: Vec::new(),
            widest: 0,
        }
    }

    /// The field of a terrain with nothing standing on it.
    pub fn from_cells(terrain: CellGrid) -> Clearance {
        let terrain_room = terrain_room_of(&terrain);
        Clearance {
            terrain,
            room: terrain_room.clone(),
            terrain_room,
            circles: Vec::new(),
            bucket_start: vec![0; BUCKETS * BUCKETS + 1],
            bucket_items: Vec::new(),
            widest: 0,
        }
    }

    /// The field of a terrain with a set of circles standing on it.
    pub fn build(terrain: CellGrid, circles: Vec<(Vec2, Fixed)>) -> Clearance {
        let mut field = Clearance::from_cells(terrain);
        field.set_circles(circles);
        field
    }

    /// A map's field with everything it starts with standing: its terrain,
    /// its buildings and its forest.
    pub fn of_map(map: &'static crate::game::MapDef) -> Clearance {
        BASES[map.index()]
            .get_or_init(|| Clearance::build(terrain_cells(map), map_circles(map)))
            .clone()
    }

    /// The terrain alone.
    pub fn terrain(&self) -> &CellGrid {
        &self.terrain
    }

    /// Every circle standing, in sorted order.
    pub fn circles(&self) -> &[(Vec2, Fixed)] {
        &self.circles
    }

    /// Replaces the circles standing with a new set, redoing the room only
    /// about the circles that came or went.
    pub fn set_circles(&mut self, mut circles: Vec<(Vec2, Fixed)>) {
        circles.sort_unstable();
        circles.dedup();
        let (mut gone, mut came) = (Vec::new(), Vec::new());
        let (mut i, mut j) = (0, 0);
        while i < self.circles.len() || j < circles.len() {
            match (self.circles.get(i), circles.get(j)) {
                (Some(old), Some(new)) if old == new => {
                    i += 1;
                    j += 1;
                }
                (Some(old), Some(new)) if old < new => {
                    gone.push(*old);
                    i += 1;
                }
                (Some(_), Some(new)) => {
                    came.push(*new);
                    j += 1;
                }
                (Some(old), None) => {
                    gone.push(*old);
                    i += 1;
                }
                (None, Some(new)) => {
                    came.push(*new);
                    j += 1;
                }
                (None, None) => break,
            }
        }
        self.circles = circles;
        self.index_circles();
        for &(at, radius) in &gone {
            let reach = radius.to_int() + CAP;
            let (lo, hi) = node_window(at, reach);
            self.recompute(lo, hi);
        }
        for &(at, radius) in &came {
            let (lo, hi) = node_window(at, radius.to_int() + CAP);
            splat(&mut self.room, at, radius, lo, hi);
        }
    }

    /// Lays the circle buckets out afresh.
    fn index_circles(&mut self) {
        self.widest = self
            .circles
            .iter()
            .map(|(_, radius)| radius.to_int())
            .max()
            .unwrap_or(0);
        let mut counts = vec![0u32; BUCKETS * BUCKETS + 1];
        for (at, _) in &self.circles {
            counts[bucket_of(*at) + 1] += 1;
        }
        for b in 1..counts.len() {
            counts[b] += counts[b - 1];
        }
        let mut fill = counts.clone();
        let mut items = vec![0u32; self.circles.len()];
        for (index, (at, _)) in self.circles.iter().enumerate() {
            let b = bucket_of(*at);
            items[fill[b] as usize] = index as u32;
            fill[b] += 1;
        }
        self.bucket_start = counts;
        self.bucket_items = items;
    }

    /// Works the room out again from scratch for every node of a window.
    fn recompute(&mut self, lo: (usize, usize), hi: (usize, usize)) {
        for ny in lo.1..=hi.1 {
            for nx in lo.0..=hi.0 {
                self.room[ny * NODES + nx] = self.terrain_room[ny * NODES + nx];
            }
        }
        let pad = rules::units(self.widest + CAP);
        let from = Clearance::node_center(lo) - Vec2 { x: pad, y: pad };
        let to = Clearance::node_center(hi) + Vec2 { x: pad, y: pad };
        let mut near = Vec::new();
        self.each_circle_in(from, to, |index| near.push(index));
        for index in near {
            let (at, radius) = self.circles[index];
            splat(&mut self.room, at, radius, lo, hi);
        }
    }

    /// Calls back with every circle whose centre lies in a box, by index
    /// into [`Clearance::circles`], bucket by bucket.
    fn each_circle_in(&self, lo: Vec2, hi: Vec2, mut each: impl FnMut(usize)) {
        let (bx0, by0) = bucket_coords(lo);
        let (bx1, by1) = bucket_coords(hi);
        for by in by0..=by1 {
            for bx in bx0..=bx1 {
                let b = by * BUCKETS + bx;
                let (start, end) = (self.bucket_start[b], self.bucket_start[b + 1]);
                for &index in &self.bucket_items[start as usize..end as usize] {
                    let (at, _) = self.circles[index as usize];
                    if at.x >= lo.x && at.x <= hi.x && at.y >= lo.y && at.y <= hi.y {
                        each(index as usize);
                    }
                }
            }
        }
    }

    /// The node a position falls into, if it is on the map.
    pub fn node_of(pos: Vec2) -> Option<(usize, usize)> {
        if pos.x.raw < 0 || pos.y.raw < 0 {
            return None;
        }
        let nx = pos.x.to_int() / NODE;
        let ny = pos.y.to_int() / NODE;
        if nx >= NODES as i32 || ny >= NODES as i32 {
            return None;
        }
        Some((nx as usize, ny as usize))
    }

    /// The centre of a node.
    pub fn node_center(node: (usize, usize)) -> Vec2 {
        Vec2::from_ints(
            node.0 as i32 * NODE + NODE / 2,
            node.1 as i32 * NODE + NODE / 2,
        )
    }

    /// The room at a node's centre, in whole world units.
    pub fn room_at(&self, node: (usize, usize)) -> i32 {
        i32::from(self.room[node.1 * NODES + node.0])
    }

    /// Whether a body of a collision size may stand at a node's centre.
    pub fn fits(&self, node: (usize, usize), radius: Fixed) -> bool {
        self.room_at(node) >= whole_units(radius)
    }

    /// Whether a position is on the map and a body of a collision size may
    /// stand at its node's centre.
    pub fn fits_at(&self, pos: Vec2, radius: Fixed) -> bool {
        Clearance::node_of(pos).is_some_and(|node| self.fits(node, radius))
    }

    /// Whether a position is on the map and on open terrain.
    pub fn walkable(&self, pos: Vec2) -> bool {
        self.terrain.open_at(pos)
    }

    /// Whether a spot is open terrain outside every circle: where a thing
    /// may be put down or come out.
    pub fn stands_clear(&self, pos: Vec2) -> bool {
        if !self.walkable(pos) {
            return false;
        }
        let pad = rules::units(self.widest);
        let mut clear = true;
        self.each_circle_in(
            pos - Vec2 { x: pad, y: pad },
            pos + Vec2 { x: pad, y: pad },
            |index| {
                let (at, radius) = self.circles[index];
                if pos.within(at, radius) {
                    clear = false;
                }
            },
        );
        clear
    }

    /// Whether a body of a collision size standing at a point is clear of
    /// every obstacle.
    pub fn point_clear(&self, at: Vec2, radius: Fixed) -> bool {
        self.segment_clear(at, at, radius, false)
    }

    /// Whether a body of a collision size can walk the straight segment
    /// clear of every obstacle.
    ///
    /// A body already inside an obstacle may walk out of it: an obstacle
    /// it overlaps where it stands does not stop a step that ends no deeper
    /// in it.
    pub fn capsule_clear(&self, from: Vec2, to: Vec2, radius: Fixed) -> bool {
        self.segment_clear(from, to, radius, true)
    }

    /// Whether a body of a collision size keeps clear of every obstacle
    /// along a segment, with or without leave to walk out of one it is
    /// already inside.
    fn segment_clear(&self, from: Vec2, to: Vec2, radius: Fixed, escape: bool) -> bool {
        let (Some(_), Some(_)) = (Clearance::node_of(from), Clearance::node_of(to)) else {
            return false;
        };
        // A body of this size is within half a node of a node centre all
        // along the segment, and every node with room above this keeps every
        // circle and closed cell a whole unit clear of the body: the exact
        // checks below can only answer yes.
        let filter = whole_units(radius) + HALF_DIAGONAL;
        if each_node_along(from, to, |node| self.room_at(node) > filter) {
            return true;
        }
        let pad = radius + rules::units(self.widest);
        let lo = Vec2 {
            x: from.x.min(to.x) - pad,
            y: from.y.min(to.y) - pad,
        };
        let hi = Vec2 {
            x: from.x.max(to.x) + pad,
            y: from.y.max(to.y) + pad,
        };
        let mut blocked = false;
        self.each_circle_in(lo, hi, |index| {
            let (at, theirs) = self.circles[index];
            if !blocked && circle_stops(at, theirs, from, to, radius, escape) {
                blocked = true;
            }
        });
        if blocked {
            return false;
        }
        let reach = whole_units(radius) + HALF_DIAGONAL;
        each_node_along(from, to, |node| {
            if self.room_at(node) >= filter {
                return true;
            }
            self.terrain_clear_about(node, reach, from, to, radius, escape)
        })
    }

    /// Whether the segment keeps a body clear of every closed cell within
    /// a reach of a node's centre.
    fn terrain_clear_about(
        &self,
        node: (usize, usize),
        reach: i32,
        from: Vec2,
        to: Vec2,
        radius: Fixed,
        escape: bool,
    ) -> bool {
        let centre = Clearance::node_center(node);
        let cells = rules::GRID_CELLS as i32;
        let cx0 = ((centre.x.to_int() - reach) / rules::GRID_CELL_SIZE).max(0);
        let cx1 = ((centre.x.to_int() + reach) / rules::GRID_CELL_SIZE).min(cells - 1);
        let cy0 = ((centre.y.to_int() - reach) / rules::GRID_CELL_SIZE).max(0);
        let cy1 = ((centre.y.to_int() + reach) / rules::GRID_CELL_SIZE).min(cells - 1);
        let need = radius.squared_raw();
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                if self.terrain.cell_open(cx as usize, cy as usize) {
                    continue;
                }
                let lo = Vec2::from_ints(cx * rules::GRID_CELL_SIZE, cy * rules::GRID_CELL_SIZE);
                let hi = Vec2::from_ints(
                    (cx + 1) * rules::GRID_CELL_SIZE,
                    (cy + 1) * rules::GRID_CELL_SIZE,
                );
                let nearest = segment_box_distance_squared(from, to, lo, hi);
                if nearest >= need {
                    continue;
                }
                // Already inside, the body may take a step that comes no
                // nearer the cell anywhere along it than where it stands.
                let at_from = point_box_distance_squared(from, lo, hi);
                if escape && at_from < need && nearest >= at_from {
                    continue;
                }
                return false;
            }
        }
        true
    }
}

/// Static obstacles for planning: the field, and any circles laid over it
/// for one plan only.
#[derive(Clone, Copy)]
pub struct Obstacles<'a> {
    /// The ground as it stands.
    pub field: &'a Clearance,
    /// Circles that stand only for this plan: bodies routed round as if
    /// they were structures.
    pub extra: &'a [(Vec2, Fixed)],
}

impl Obstacles<'_> {
    /// Whether a body of a collision size may stand at a node's centre.
    pub fn fits(&self, node: (usize, usize), radius: Fixed) -> bool {
        self.field.fits(node, radius) && {
            let centre = Clearance::node_center(node);
            self.extra
                .iter()
                .all(|&(at, theirs)| !centre.within(at, theirs + radius))
        }
    }

    /// Whether a body of a collision size can walk the straight segment.
    pub fn clear(&self, from: Vec2, to: Vec2, radius: Fixed) -> bool {
        self.field.capsule_clear(from, to, radius)
            && self
                .extra
                .iter()
                .all(|&(at, theirs)| !circle_stops(at, theirs, from, to, radius, true))
    }

    /// Whether a body of a collision size standing at a point is clear.
    pub fn point_clear(&self, at: Vec2, radius: Fixed) -> bool {
        self.field.point_clear(at, radius)
            && self
                .extra
                .iter()
                .all(|&(spot, theirs)| !circle_stops(spot, theirs, at, at, radius, false))
    }
}

/// Whether a circle stops a body of a collision size walking a segment.
///
/// With leave to escape, a body already inside the circle where it stands
/// is let out of it: the circle stops only a step that comes nearer its
/// centre somewhere along it than where the body stands.
pub fn circle_stops(
    at: Vec2,
    theirs: Fixed,
    from: Vec2,
    to: Vec2,
    radius: Fixed,
    escape: bool,
) -> bool {
    let need = (theirs + radius).squared_raw();
    let nearest = segment_distance_squared(at, from, to);
    if nearest >= need {
        return false;
    }
    let at_from = from.distance_squared(at);
    !(escape && at_from < need && nearest >= at_from)
}

/// A radius in whole world units, rounded up.
fn whole_units(radius: Fixed) -> i32 {
    ((i64::from(radius.raw) + ((1 << Fixed::FRAC_BITS) - 1)) >> Fixed::FRAC_BITS) as i32
}

/// The nodes a square window about a position covers, clamped to the map.
fn node_window(at: Vec2, reach: i32) -> ((usize, usize), (usize, usize)) {
    let last = NODES as i32 - 1;
    let x0 = ((at.x.to_int() - reach) / NODE).clamp(0, last);
    let x1 = ((at.x.to_int() + reach) / NODE).clamp(0, last);
    let y0 = ((at.y.to_int() - reach) / NODE).clamp(0, last);
    let y1 = ((at.y.to_int() + reach) / NODE).clamp(0, last);
    ((x0 as usize, y0 as usize), (x1 as usize, y1 as usize))
}

/// Lowers the room of every node of a window to its distance from a circle.
fn splat(room: &mut [u8], at: Vec2, radius: Fixed, lo: (usize, usize), hi: (usize, usize)) {
    let theirs = i64::from(radius.to_int());
    let reach = rules::units(radius.to_int() + CAP).squared_raw();
    for ny in lo.1..=hi.1 {
        for nx in lo.0..=hi.0 {
            let centre = Clearance::node_center((nx, ny));
            let apart = centre.distance_squared(at);
            if apart >= reach {
                continue;
            }
            let far = ((isqrt64(apart) >> Fixed::FRAC_BITS) - theirs).max(0);
            let slot = &mut room[ny * NODES + nx];
            *slot = (*slot).min(far as u8);
        }
    }
}

/// The bucket a position's centre falls in.
fn bucket_of(at: Vec2) -> usize {
    let (bx, by) = bucket_coords(at);
    by * BUCKETS + bx
}

/// The bucket coordinates of a position, clamped to the map.
fn bucket_coords(at: Vec2) -> (usize, usize) {
    let last = BUCKETS as i32 - 1;
    let bx = (at.x.to_int() / rules::BUCKET_SIZE).clamp(0, last);
    let by = (at.y.to_int() / rules::BUCKET_SIZE).clamp(0, last);
    (bx as usize, by as usize)
}

/// The squared distance from a node's centre to a closed node square that
/// stands `k` nodes away along one axis, in whole units.
fn axis_gap_squared(k: i32) -> u32 {
    let gap = (NODE * k.abs() - NODE / 2).max(0) as u32;
    gap * gap
}

/// The room at every node with only a terrain standing: the distance from
/// each node's centre to the nearest closed cell, the map's edge counted as
/// closed, capped.
fn terrain_room_of(terrain: &CellGrid) -> Vec<u8> {
    let cell_nodes = (rules::GRID_CELL_SIZE / NODE) as usize;
    let closed = |nx: usize, ny: usize| !terrain.cell_open(nx / cell_nodes, ny / cell_nodes);
    let n = NODES as i32;
    // Along each row: the squared gap to the nearest closed node in the
    // row, the edge counted as closed just past either end.
    let mut rows = vec![0u32; NODES * NODES];
    for ny in 0..NODES {
        let mut last = -1;
        for nx in 0..NODES {
            if closed(nx, ny) {
                last = nx as i32;
            }
            rows[ny * NODES + nx] = axis_gap_squared(nx as i32 - last);
        }
        let mut next = n;
        for nx in (0..NODES).rev() {
            if closed(nx, ny) {
                next = nx as i32;
            }
            let slot = &mut rows[ny * NODES + nx];
            *slot = (*slot).min(axis_gap_squared(next - nx as i32));
        }
    }
    // Down each column: the row gaps combined with the gap across rows.
    let cap = (CAP * CAP) as u32;
    let mut span = 1;
    while axis_gap_squared(span) < cap {
        span += 1;
    }
    let mut room = vec![0u8; NODES * NODES];
    for nx in 0..NODES {
        for ny in 0..NODES {
            let mut best = rows[ny * NODES + nx]
                .min(axis_gap_squared(ny as i32 + 1))
                .min(axis_gap_squared(n - ny as i32));
            for dy in 1..span {
                let across = axis_gap_squared(dy);
                if ny >= dy as usize {
                    best = best.min(rows[(ny - dy as usize) * NODES + nx] + across);
                }
                if ny + (dy as usize) < NODES {
                    best = best.min(rows[(ny + dy as usize) * NODES + nx] + across);
                }
            }
            room[ny * NODES + nx] = isqrt64(i64::from(best)).min(i64::from(CAP)) as u8;
        }
    }
    room
}

/// Calls back for every node a segment passes through, from its first to
/// its last, stopping early when the callback says so. Answers whether it
/// got to the end.
fn each_node_along(from: Vec2, to: Vec2, mut each: impl FnMut((usize, usize)) -> bool) -> bool {
    let (Some(start), Some(end)) = (Clearance::node_of(from), Clearance::node_of(to)) else {
        return false;
    };
    let size = i64::from(NODE) << Fixed::FRAC_BITS;
    let (ax, ay) = (i64::from(from.x.raw), i64::from(from.y.raw));
    let dx = i64::from(to.x.raw) - ax;
    let dy = i64::from(to.y.raw) - ay;
    let (mut x, mut y) = (start.0 as i64, start.1 as i64);
    if !each((x as usize, y as usize)) {
        return false;
    }
    let mut left = 2 * ((end.0 as i64 - x).abs() + (end.1 as i64 - y).abs()) + 2;
    while (x as usize, y as usize) != end && left > 0 {
        left -= 1;
        let to_x = match dx.signum() {
            1 => (x + 1) * size - ax,
            -1 => ax - x * size,
            _ => i64::MAX,
        };
        let to_y = match dy.signum() {
            1 => (y + 1) * size - ay,
            -1 => ay - y * size,
            _ => i64::MAX,
        };
        let (step_x, step_y) = if dx == 0 {
            (false, true)
        } else if dy == 0 {
            (true, false)
        } else {
            let tx = i128::from(to_x) * i128::from(dy.abs());
            let ty = i128::from(to_y) * i128::from(dx.abs());
            (tx <= ty, ty <= tx)
        };
        if step_x {
            x += dx.signum();
        }
        if step_y {
            y += dy.signum();
        }
        if x < 0 || y < 0 || x >= NODES as i64 || y >= NODES as i64 {
            return false;
        }
        if !each((x as usize, y as usize)) {
            return false;
        }
    }
    true
}

/// A map's terrain as cells.
fn terrain_cells(map: &crate::game::MapDef) -> CellGrid {
    let ground = crate::game::Ground::of(map);
    let mut cells = CellGrid::open();
    for cy in 0..rules::GRID_CELLS {
        for cx in 0..rules::GRID_CELLS {
            if !ground.cell_walkable(cx, cy) {
                cells.close_cell(cx, cy);
            }
        }
    }
    cells
}

/// Every circle a map starts with: its buildings and its forest, at their
/// collision sizes.
fn map_circles(map: &crate::game::MapDef) -> Vec<(Vec2, Fixed)> {
    let mut circles = Vec::new();
    for at in map.fountains {
        circles.push((at, rules::units(rules::FOUNTAIN_COLLISION)));
    }
    for (team, at) in [bota_proto::Team::Radiant, bota_proto::Team::Dire]
        .into_iter()
        .zip(map.ancients)
    {
        if let Some(at) = at {
            circles.push((at, rules::units(crate::game::ancient_of(team).collision)));
        }
    }
    for &(_, _, at) in map.radiant_towers.iter().chain(map.dire_towers) {
        circles.push((at, rules::units(rules::TOWER_COLLISION)));
    }
    for &(_, _, at) in map.barracks[0].iter().chain(map.barracks[1]) {
        circles.push((at, rules::units(rules::RAX_COLLISION)));
    }
    for at in crate::game::tree_positions(map) {
        circles.push((at, rules::units(rules::TREE_RADIUS)));
    }
    circles
}
