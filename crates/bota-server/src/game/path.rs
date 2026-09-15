//! Routes over the walking lattice: A* round what stands still, the corners
//! then drawn tight against what they round.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bota_proto::{Fixed, Vec2};

use crate::game::{Clearance, Obstacles, isqrt64, point_along, rules};

const NODES: usize = rules::WALK_CELLS;
const STRAIGHT: u32 = 100;
const DIAGONAL: u32 = 141;

/// How many nodes out a node with room is looked for: across the widest
/// footprint on any map and the widest body, and one more.
const OPEN_SEARCH_NODES: i32 =
    (rules::DIRE_ANCIENT_COLLISION + rules::STEER_MARGIN + rules::WIDEST_MARCHER)
        / rules::WALK_CELL_SIZE
        + 2;

/// The room a route keeps for a body: its collision size and the margin.
pub fn plan_radius(collision: Fixed) -> Fixed {
    collision + rules::units(rules::STEER_MARGIN)
}

/// The scratch an A* search over the lattice works in, kept between
/// searches so none of them starts by clearing the whole map.
pub struct Planner {
    /// The cost got to each node at, for nodes stamped with the epoch.
    g: Vec<u32>,
    /// The node each node was got to from, for nodes stamped with the
    /// epoch. `u32::MAX` for the start.
    parent: Vec<u32>,
    /// The search that last wrote each node.
    stamp: Vec<u32>,
    /// The search running.
    epoch: u32,
    /// The open list: cost plus the estimate left, then the node.
    heap: BinaryHeap<Reverse<(u32, u32)>>,
}

impl Default for Planner {
    fn default() -> Self {
        Planner::new()
    }
}

impl Planner {
    /// A planner that has searched nothing.
    pub fn new() -> Planner {
        Planner {
            g: vec![0; NODES * NODES],
            parent: vec![u32::MAX; NODES * NODES],
            stamp: vec![0; NODES * NODES],
            epoch: 0,
            heap: BinaryHeap::new(),
        }
    }

    /// A* over the lattice for a body of a collision size, returning the
    /// corners of the walk, the last of them where the walk ends: at `to`
    /// when it can be stood on and reached, else at the open spot nearest
    /// to it that can, on the walker's own side of whatever shuts it.
    ///
    /// Empty when the walk ends in the node the walker stands in. Diagonal
    /// steps never cut a blocked corner. Ties break on node index, so the
    /// route is the same on every platform.
    pub fn find_path(
        &mut self,
        ob: &Obstacles,
        from: Vec2,
        to: Vec2,
        collision: Fixed,
    ) -> Vec<Vec2> {
        self.find_path_within(ob, from, to, collision, rules::PATH_EXPANSIONS)
    }

    /// [`Planner::find_path`] with its own budget of nodes to expand.
    pub fn find_path_within(
        &mut self,
        ob: &Obstacles,
        from: Vec2,
        to: Vec2,
        collision: Fixed,
        budget: u32,
    ) -> Vec<Vec2> {
        let room = plan_radius(collision);
        let (Some(start), Some(asked)) = (Clearance::node_of(from), Clearance::node_of(to)) else {
            return Vec::new();
        };
        let (Some(start), Some(goal)) = (
            routable_node(ob, start, room),
            node_beside(ob, to, from, room).or_else(|| routable_node(ob, asked, room)),
        ) else {
            return Vec::new();
        };
        if start == goal {
            return Vec::new();
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.stamp.fill(0);
            self.epoch = 1;
        }
        let epoch = self.epoch;
        self.heap.clear();
        let idx = |c: (usize, usize)| c.1 * NODES + c.0;
        self.g[idx(start)] = 0;
        self.parent[idx(start)] = u32::MAX;
        self.stamp[idx(start)] = epoch;
        self.heap
            .push(Reverse((heuristic(start, goal), idx(start) as u32)));
        // The node got to that lies nearest the goal, for when no way leads
        // there.
        let mut nearest = (heuristic(start, goal), idx(start));
        let mut expanded = 0;
        while let Some(Reverse((f, at))) = self.heap.pop() {
            let at = at as usize;
            let node = (at % NODES, at / NODES);
            let g = self.g[at];
            if f > g + heuristic(node, goal) {
                continue; // an entry left behind by a better way to the node
            }
            if node == goal {
                break;
            }
            // Past the budget the walk goes to the nearest node got to.
            if expanded >= budget {
                break;
            }
            expanded += 1;
            let left = heuristic(node, goal);
            if left < nearest.0 {
                nearest = (left, at);
            }
            for (i, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
                let nx = node.0 as i32 + dx;
                let ny = node.1 as i32 + dy;
                if nx < 0 || ny < 0 || nx as usize >= NODES || ny as usize >= NODES {
                    continue;
                }
                let next = (nx as usize, ny as usize);
                if !ob.fits(next, room) {
                    continue;
                }
                let diagonal = i >= 4;
                if diagonal
                    && (!ob.fits((next.0, node.1), room) || !ob.fits((node.0, next.1), room))
                {
                    continue; // no cutting a blocked corner
                }
                let cost = g + if diagonal { DIAGONAL } else { STRAIGHT };
                let ni = idx(next);
                if self.stamp[ni] != epoch || cost < self.g[ni] {
                    self.stamp[ni] = epoch;
                    self.g[ni] = cost;
                    self.parent[ni] = at as u32;
                    self.heap
                        .push(Reverse((cost + heuristic(next, goal), ni as u32)));
                }
            }
        }
        let end = if self.stamp[idx(goal)] == epoch && expanded < budget {
            idx(goal)
        } else {
            nearest.1
        };
        if end == idx(start) {
            return Vec::new();
        }
        let goal = (end % NODES, end / NODES);
        // Walk the parents back, then keep only the corners.
        let mut nodes = vec![goal];
        let mut at = end;
        while at != idx(start) {
            at = self.parent[at] as usize;
            nodes.push((at % NODES, at / NODES));
        }
        nodes.reverse();
        let mut corners: Vec<(usize, usize)> = Vec::new();
        for i in 2..nodes.len() {
            let dir = (
                nodes[i].0 as i32 - nodes[i - 1].0 as i32,
                nodes[i].1 as i32 - nodes[i - 1].1 as i32,
            );
            let prev_dir = (
                nodes[i - 1].0 as i32 - nodes[i - 2].0 as i32,
                nodes[i - 1].1 as i32 - nodes[i - 2].1 as i32,
            );
            if prev_dir != dir {
                corners.push(nodes[i - 1]);
            }
        }
        corners.push(goal);
        let mut spots: Vec<Vec2> = corners.iter().map(|&c| Clearance::node_center(c)).collect();
        // The walk ends on the spot asked for itself when its own node was
        // got to, or when the spot is in a straight line from the node the
        // walk got to: a spot may be stood on though its node's centre may
        // not.
        if goal == asked || ob.clear(Clearance::node_center(goal), to, room) {
            *spots.last_mut().expect("the end is kept") = to;
        }
        let mut spots = pull_string(ob, from, spots, room);
        tighten(ob, from, &mut spots, room);
        pull_string(ob, from, spots, room)
    }
}

/// A* with a planner used once, for routes laid outside a match's walking.
pub fn find_path(field: &Clearance, from: Vec2, to: Vec2, collision: Fixed) -> Vec<Vec2> {
    let ob = Obstacles { field, extra: &[] };
    Planner::new().find_path(&ob, from, to, collision)
}

fn heuristic(a: (usize, usize), b: (usize, usize)) -> u32 {
    let dx = a.0.abs_diff(b.0) as u32;
    let dy = a.1.abs_diff(b.1) as u32;
    STRAIGHT * dx.max(dy) + (DIAGONAL - STRAIGHT) * dx.min(dy)
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

/// The spot a body of a collision size may actually stand on beside a point
/// that may sit inside a building's footprint: the point itself when there
/// is room for it, else the centre of the first node with room on the way
/// out of the footprint towards `toward`.
///
/// Falls back on the nearest node with room in any direction when that way
/// out is shut too.
pub fn open_beside(ob: &Obstacles, at: Vec2, toward: Vec2, collision: Fixed) -> Vec2 {
    let room = plan_radius(collision);
    if Clearance::node_of(at).is_some_and(|node| ob.fits(node, room)) {
        return at;
    }
    node_beside(ob, at, toward, room)
        .or_else(|| Clearance::node_of(at).and_then(|node| routable_node(ob, node, room)))
        .map_or(at, Clearance::node_center)
}

/// The first node with room for a body on the way from a point towards
/// another, the point's own node included, within [`OPEN_SEARCH_NODES`].
/// None with the whole way shut, or the two points one.
fn node_beside(ob: &Obstacles, at: Vec2, toward: Vec2, room: Fixed) -> Option<(usize, usize)> {
    let dx = i64::from(toward.x.raw) - i64::from(at.x.raw);
    let dy = i64::from(toward.y.raw) - i64::from(at.y.raw);
    let len = dx.abs().max(dy.abs());
    if len == 0 {
        return None;
    }
    let sample = i64::from(rules::WALK_CELL_SIZE) << (Fixed::FRAC_BITS - 1); // half a node, raw
    let steps = i64::from(OPEN_SEARCH_NODES) * 2;
    for step in 0..=steps {
        let p = Vec2 {
            x: Fixed {
                raw: (i64::from(at.x.raw) + dx * sample * step / len) as i32,
            },
            y: Fixed {
                raw: (i64::from(at.y.raw) + dy * sample * step / len) as i32,
            },
        };
        if let Some(node) = Clearance::node_of(p)
            && ob.fits(node, room)
        {
            return Some(node);
        }
    }
    None
}

/// The node to route a body to for a goal: the goal node itself, or the
/// node with room nearest to it when there is none there.
///
/// Ties break on the lower row, then the lower column. None when nothing
/// within [`OPEN_SEARCH_NODES`] has room.
fn routable_node(ob: &Obstacles, node: (usize, usize), room: Fixed) -> Option<(usize, usize)> {
    if ob.fits(node, room) {
        return Some(node);
    }
    let mut best: Option<(i32, (usize, usize))> = None;
    for ring in 1..=OPEN_SEARCH_NODES {
        if best.is_some_and(|(had, _)| ring * ring > had) {
            break;
        }
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let nx = node.0 as i32 + dx;
                let ny = node.1 as i32 + dy;
                if nx < 0 || ny < 0 || nx as usize >= NODES || ny as usize >= NODES {
                    continue;
                }
                if !ob.fits((nx as usize, ny as usize), room) {
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

/// The corners a walk keeps: from where it stands and from each corner kept,
/// the walk goes straight to the farthest later corner the body can reach
/// in a line, so a route that stepped round a footprint node by node rounds
/// it in a few straight legs. The last corner is always kept.
fn pull_string(ob: &Obstacles, from: Vec2, corners: Vec<Vec2>, room: Fixed) -> Vec<Vec2> {
    let mut kept = Vec::with_capacity(corners.len());
    let mut anchor = from;
    let mut at = 0;
    while at < corners.len() {
        let mut far = at;
        for (later, &corner) in corners.iter().enumerate().skip(at + 1) {
            if ob.clear(anchor, corner, room) {
                far = later;
            }
        }
        anchor = corners[far];
        kept.push(anchor);
        at = far + 1;
    }
    kept
}

/// Draws every corner but the last in towards the obstacle it rounds: along
/// the bisector of its two legs, as far as both legs stay clear, up to
/// [`rules::TIGHTEN_MAX`].
fn tighten(ob: &Obstacles, from: Vec2, corners: &mut [Vec2], room: Fixed) {
    let most = i64::from(rules::units(rules::TIGHTEN_MAX).raw);
    for i in 0..corners.len().saturating_sub(1) {
        let prev = if i == 0 { from } else { corners[i - 1] };
        let next = corners[i + 1];
        let corner = corners[i];
        let Some(inward) = bisector(prev, corner, next) else {
            continue;
        };
        let (mut clear, mut shut) = (0i64, most + 1);
        while shut - clear > 1 {
            let mid = (clear + shut) / 2;
            let moved = point_along(corner, corner + inward, Fixed { raw: mid as i32 });
            if ob.clear(prev, moved, room) && ob.clear(moved, next, room) {
                clear = mid;
            } else {
                shut = mid;
            }
        }
        if clear > 0 {
            corners[i] = point_along(corner, corner + inward, Fixed { raw: clear as i32 });
        }
    }
}

/// An offset from a corner along the bisector of its two legs, pointing
/// into the turn. None when the legs run straight through.
fn bisector(prev: Vec2, corner: Vec2, next: Vec2) -> Option<Vec2> {
    let unit = |to: Vec2| {
        let dx = i64::from(to.x.raw) - i64::from(corner.x.raw);
        let dy = i64::from(to.y.raw) - i64::from(corner.y.raw);
        let len = isqrt64(dx * dx + dy * dy);
        if len == 0 {
            return (0, 0);
        }
        let scale = i64::from(rules::units(4096).raw);
        (dx * scale / len, dy * scale / len)
    };
    let (ux, uy) = unit(prev);
    let (vx, vy) = unit(next);
    let (bx, by) = (ux + vx, uy + vy);
    if bx == 0 && by == 0 {
        return None;
    }
    Some(Vec2 {
        x: Fixed { raw: bx as i32 },
        y: Fixed { raw: by as i32 },
    })
}

/// The point a reach along a polyline from a position, through its points
/// in order: the polyline's last point when it is shorter than the reach.
pub fn along_polyline(from: Vec2, points: &[Vec2], reach: Fixed) -> Vec2 {
    let mut at = from;
    let mut left = i64::from(reach.raw);
    for &point in points {
        let leg = isqrt64(at.distance_squared(point));
        if leg >= left {
            return point_along(at, point, Fixed { raw: left as i32 });
        }
        left -= leg;
        at = point;
    }
    at
}
