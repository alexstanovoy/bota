//! Routes over the walking lattice: A* round what stands still, the corners
//! then drawn tight against what they round.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bota_core::{isqrt64, point_along, rules};
use bota_proto::{Fixed, Vec2};

use crate::game::{Clearance, Obstacles};

const NODES: usize = rules::WALK_CELLS;
const NODE_SIZE: i32 = rules::WALK_CELL_SIZE;
const STRAIGHT: u32 = 100;
const DIAGONAL: u32 = 141;

/// How many nodes out a node with room is looked for: the Dire ancient's
/// footprint, the steer margin and the widest marcher, in nodes, plus two.
const OPEN_SEARCH_NODES: i32 =
    (rules::DIRE_ANCIENT_COLLISION + rules::STEER_MARGIN + rules::WIDEST_MARCHER)
        / rules::WALK_CELL_SIZE
        + 2;

/// The index of a node in the planner's arrays.
fn idx(node: (usize, usize)) -> usize {
    node.1 * NODES + node.0
}

/// The node an offset away from another, while it lies on the lattice.
fn offset_node(node: (usize, usize), dx: i32, dy: i32) -> Option<(usize, usize)> {
    let x = usize::try_from(node.0 as i32 + dx).ok()?;
    let y = usize::try_from(node.1 as i32 + dy).ok()?;
    (x < NODES && y < NODES).then_some((x, y))
}

/// The room a route keeps for a body: its collision size and the margin.
pub fn plan_radius(collision: Fixed) -> Fixed {
    collision + rules::units(rules::STEER_MARGIN)
}

/// What a search knows of one node.
#[derive(Clone, Copy)]
struct Node {
    /// The cost got to it at, when stamped with the running search.
    g: u32,
    /// The node it was got to from, when stamped with the running search.
    /// `u32::MAX` for the start.
    parent: u32,
    /// The search that last wrote it.
    stamp: u32,
}

impl Node {
    /// A node no search has written.
    const UNSEEN: Node = Node {
        g: 0,
        parent: u32::MAX,
        stamp: 0,
    };
}

/// The scratch an A* search over the lattice works in, kept between
/// searches so none of them starts by clearing the whole map.
#[derive(Clone)]
pub struct Planner {
    /// What the searches know of each node, side by side: a search reads
    /// and writes a node's cost, parent and stamp together.
    nodes: Vec<Node>,
    /// The search running.
    epoch: u32,
    /// The open list.
    open: OpenList,
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
            nodes: vec![Node::UNSEEN; NODES * NODES],
            epoch: 0,
            open: OpenList::new(),
        }
    }

    /// A* over the lattice for a body of a collision size, returning the
    /// corners of the walk, the last of them where the walk ends: at `to`
    /// when it can be stood on and reached, else at the open spot nearest
    /// to it that can, on the walker's own side of whatever shuts it.
    ///
    /// The walk keeps [`plan_radius`] off everything, or only the body's
    /// own collision size when the margin shuts the walker in.
    ///
    /// Empty when the walk ends in the node the walker stands in or no node
    /// with room is found at either end. Diagonal steps never cut a blocked
    /// corner. Ties break on node index.
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
        match self.walk_keeping(ob, from, to, plan_radius(collision), collision, budget) {
            (spots, false) => spots,
            (_, true) => {
                self.walk_keeping(ob, from, to, collision, collision, budget)
                    .0
            }
        }
    }

    /// The corners of a walk that keeps a room off everything, as
    /// [`Planner::find_path`] answers them, and whether the room shuts the
    /// walker in: no node with room at either end, or every node got to
    /// expanded within the budget and none of them the goal.
    ///
    /// The walk sets out from where the body (of collision size `body`)
    /// stands to a node it gets to in a straight line: the nearest with
    /// room, and any other with room about the node it stands in. A node
    /// with room beyond a row of trees is no start for one standing between
    /// them, and one standing in a gap between two trees steps straight
    /// out of it, whichever way the lattice's steps would leave it.
    fn walk_keeping(
        &mut self,
        ob: &Obstacles,
        from: Vec2,
        to: Vec2,
        room: Fixed,
        body: Fixed,
        budget: u32,
    ) -> (Vec<Vec2>, bool) {
        let (Some(stands), Some(asked)) = (Clearance::node_of(from), Clearance::node_of(to)) else {
            return (Vec::new(), false);
        };
        let reached = |node| ob.clear(from, Clearance::node_center(node), body);
        let (Some(start), Some(goal)) = (
            nearest_node(ob, stands, room, reached),
            node_beside(ob, to, from, room).or_else(|| routable_node(ob, asked, room)),
        ) else {
            return (Vec::new(), true);
        };
        if start == goal {
            return (Vec::new(), false);
        }
        let mut seeds = vec![start];
        for (dx, dy) in NEIGHBOURS {
            if let Some(near) = offset_node(stands, dx, dy)
                .filter(|&near| near != start && ob.fits(near, room) && reached(near))
            {
                seeds.push(near);
            }
        }
        let (end, shut) = self.search(ob, from, &seeds, goal, room, budget);
        if end == idx(start) {
            return (Vec::new(), shut);
        }
        let goal = (end % NODES, end / NODES);
        let corners = self.corners(end);
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
        (pull_string(ob, from, spots, room), shut)
    }

    /// Runs A* from where a body stands, through the nodes it sets out from
    /// (each at the cost of the straight way to it), towards a goal node
    /// within a budget of nodes to expand, and answers the node the walk
    /// ends in: the goal when it was got to within the budget, else the node
    /// got to that lies nearest it. Ties break on node index. Answers too
    /// whether every node got to was expanded within the budget without
    /// getting to the goal.
    fn search(
        &mut self,
        ob: &Obstacles,
        from: Vec2,
        seeds: &[(usize, usize)],
        goal: (usize, usize),
        room: Fixed,
        budget: u32,
    ) -> (usize, bool) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.nodes.fill(Node::UNSEEN);
            self.epoch = 1;
        }
        let epoch = self.epoch;
        self.open.clear();
        let mut nearest = (u32::MAX, idx(seeds[0]));
        for &seed in seeds {
            let i = idx(seed);
            let leg = isqrt64(from.distance_squared(Clearance::node_center(seed)));
            let g = (leg * i64::from(STRAIGHT) / i64::from(rules::units(NODE_SIZE).raw)) as u32;
            if self.nodes[i].stamp == epoch && self.nodes[i].g <= g {
                continue;
            }
            self.nodes[i].g = g;
            self.nodes[i].parent = u32::MAX;
            self.nodes[i].stamp = epoch;
            self.open.push(g + heuristic(seed, goal), i);
            nearest = nearest.min((heuristic(seed, goal), i));
        }
        let mut expanded = 0;
        let mut shut = true;
        while let Some((f, at)) = self.open.pop() {
            let node = (at % NODES, at / NODES);
            let g = self.nodes[at].g;
            if f > g + heuristic(node, goal) {
                continue; // an entry left behind by a better way to the node
            }
            if node == goal || expanded >= budget {
                shut = false;
                break;
            }
            expanded += 1;
            let left = heuristic(node, goal);
            if left < nearest.0 {
                nearest = (left, at);
            }
            self.expand(ob, node, goal, room);
        }
        if self.nodes[idx(goal)].stamp == epoch && expanded < budget {
            (idx(goal), false)
        } else {
            (nearest.1, shut)
        }
    }

    /// Opens every neighbour of a node a body of a room fits in, never
    /// cutting a blocked corner, when it is got to cheaper than before.
    fn expand(&mut self, ob: &Obstacles, node: (usize, usize), goal: (usize, usize), room: Fixed) {
        let (at, g, epoch) = (idx(node), self.nodes[idx(node)].g, self.epoch);
        for (i, (dx, dy)) in NEIGHBOURS.iter().enumerate() {
            let Some(next) = offset_node(node, *dx, *dy).filter(|next| ob.fits(*next, room)) else {
                continue;
            };
            let diagonal = i >= 4;
            if diagonal && (!ob.fits((next.0, node.1), room) || !ob.fits((node.0, next.1), room)) {
                continue;
            }
            let cost = g + if diagonal { DIAGONAL } else { STRAIGHT };
            let ni = idx(next);
            if self.nodes[ni].stamp != epoch || cost < self.nodes[ni].g {
                self.nodes[ni].stamp = epoch;
                self.nodes[ni].g = cost;
                self.nodes[ni].parent = at as u32;
                self.open.push(cost + heuristic(next, goal), ni);
            }
        }
    }

    /// The nodes of the last search's walk where the way turns, the node it
    /// set out from first and the node it ends in last. The first is pulled
    /// out of the walk ([`pull_string`]) unless the way beyond it is not in a
    /// straight line from where the body stands.
    fn corners(&self, end: usize) -> Vec<(usize, usize)> {
        let mut nodes = vec![(end % NODES, end / NODES)];
        let mut at = end;
        while self.nodes[at].parent != u32::MAX {
            at = self.nodes[at].parent as usize;
            nodes.push((at % NODES, at / NODES));
        }
        nodes.reverse();
        let step = |a: (usize, usize), b: (usize, usize)| {
            (b.0 as i32 - a.0 as i32, b.1 as i32 - a.1 as i32)
        };
        let mut corners = vec![nodes[0]];
        corners.extend(
            nodes
                .windows(3)
                .filter(|w| step(w[0], w[1]) != step(w[1], w[2]))
                .map(|w| w[1]),
        );
        if nodes.len() > 1 {
            corners.push(nodes[nodes.len() - 1]);
        }
        corners
    }
}

/// A* with a planner used once, for routes laid outside a match's walking.
pub fn find_path(field: &Clearance, from: Vec2, to: Vec2, collision: Fixed) -> Vec<Vec2> {
    let ob = Obstacles { field, extra: &[] };
    Planner::new().find_path(&ob, from, to, collision)
}

/// Costs plus estimates the open list holds apart beyond the one it is
/// taking: above the most a step raises one, a diagonal's cost plus the
/// most its estimate can fall by, twice [`DIAGONAL`].
const OPEN_SPAN: u32 = 512;

const _: () = assert!(2 * DIAGONAL < OPEN_SPAN);

/// The open list of a search, handing out its entries least cost plus
/// estimate first, then least node index: the order of a binary heap of the
/// pairs, so every search expands the same nodes in the same order.
///
/// The estimate is consistent (a step never lowers the cost plus the
/// estimate), so nothing put on the list once it is being taken from comes
/// below the cost plus estimate being taken, or [`OPEN_SPAN`] or more above
/// it. Each cost plus estimate within the span keeps its nodes unsorted in a
/// bucket of its own and only the one being taken is kept as a heap. The
/// seeds, put on before anything is taken, may lie further apart: those
/// beyond the span wait in a heap of their own until it reaches them.
#[derive(Clone)]
struct OpenList {
    /// The nodes of each cost plus estimate within the span above the one
    /// being taken, by its remainder over [`OPEN_SPAN`].
    ahead: Vec<Vec<u32>>,
    /// The nodes at the cost plus estimate being taken, least first.
    taking: BinaryHeap<Reverse<u32>>,
    /// The entries put on before anything was taken.
    seeds: Vec<(u32, u32)>,
    /// The seeds beyond the span, least first.
    beyond: BinaryHeap<Reverse<(u32, u32)>>,
    /// The cost plus estimate being taken.
    at: u32,
    /// Entries held in `ahead` and `taking`.
    held: usize,
    /// Whether anything has been taken since the list was last cleared.
    taken: bool,
}

impl OpenList {
    /// An empty list.
    fn new() -> OpenList {
        OpenList {
            ahead: vec![Vec::new(); OPEN_SPAN as usize],
            taking: BinaryHeap::new(),
            seeds: Vec::new(),
            beyond: BinaryHeap::new(),
            at: 0,
            held: 0,
            taken: false,
        }
    }

    /// Forgets every entry, keeping the room.
    fn clear(&mut self) {
        if self.held > 0 {
            self.ahead.iter_mut().for_each(Vec::clear);
            self.taking.clear();
        }
        self.seeds.clear();
        self.beyond.clear();
        (self.at, self.held, self.taken) = (0, 0, false);
    }

    /// Puts a node on at a cost plus estimate.
    fn push(&mut self, f: u32, node: usize) {
        let node = u32::try_from(node).expect("a lattice index fits u32");
        if !self.taken {
            self.seeds.push((f, node));
            return;
        }
        assert!(
            f >= self.at && f - self.at < OPEN_SPAN,
            "an A* step moved the cost plus estimate out of the open list's span"
        );
        self.hold(f, node);
    }

    /// Keeps a node within the span.
    fn hold(&mut self, f: u32, node: u32) {
        if f == self.at {
            self.taking.push(Reverse(node));
        } else {
            self.ahead[(f % OPEN_SPAN) as usize].push(node);
        }
        self.held += 1;
    }

    /// Takes the entry of least cost plus estimate, the least node of those.
    fn pop(&mut self) -> Option<(u32, usize)> {
        if !self.taken {
            self.taken = true;
            self.at = self.seeds.iter().map(|&(f, _)| f).min()?;
            for (f, node) in std::mem::take(&mut self.seeds) {
                if f - self.at < OPEN_SPAN {
                    self.hold(f, node);
                } else {
                    self.beyond.push(Reverse((f, node)));
                }
            }
        }
        loop {
            if let Some(Reverse(node)) = self.taking.pop() {
                self.held -= 1;
                return Some((self.at, node as usize));
            }
            if self.held == 0 && self.beyond.is_empty() {
                return None;
            }
            let bucket = &mut self.ahead[(self.at % OPEN_SPAN) as usize];
            if !bucket.is_empty() {
                self.taking.extend(bucket.drain(..).map(Reverse));
                continue;
            }
            self.at += 1;
            while let Some(&Reverse((f, node))) = self.beyond.peek()
                && f - self.at < OPEN_SPAN
            {
                self.beyond.pop();
                self.hold(f, node);
            }
        }
    }
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
/// Ties break on the inner ring, then the lower row, then the lower
/// column. None when nothing within [`OPEN_SEARCH_NODES`] has room.
fn routable_node(ob: &Obstacles, node: (usize, usize), room: Fixed) -> Option<(usize, usize)> {
    nearest_node(ob, node, room, |_| true)
}

/// [`routable_node`] among the nodes a test lets through.
fn nearest_node(
    ob: &Obstacles,
    node: (usize, usize),
    room: Fixed,
    lets: impl Fn((usize, usize)) -> bool,
) -> Option<(usize, usize)> {
    if ob.fits(node, room) && lets(node) {
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
                let Some(near) =
                    offset_node(node, dx, dy).filter(|near| ob.fits(*near, room) && lets(*near))
                else {
                    continue;
                };
                let apart = dx * dx + dy * dy;
                if best.is_none_or(|(had, _)| apart < had) {
                    best = Some((apart, near));
                }
            }
        }
    }
    best.map(|(_, found)| found)
}

/// The corners a walk keeps: from where it stands and from each corner kept,
/// the walk goes straight to the farthest later corner the body can reach
/// in a line. The last corner is always kept.
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

#[cfg(test)]
mod tests {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    use super::{DIAGONAL, OPEN_SPAN, OpenList};

    /// The open list hands its entries out in the order a binary heap of the
    /// pairs would, whatever the seeds' spread, as A* puts them on: steps
    /// never below what is being taken and within twice a diagonal above it.
    #[test]
    fn the_open_list_hands_out_what_a_heap_of_the_pairs_would() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut draw = |below: u32| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % u64::from(below)) as u32
        };
        let mut open = OpenList::new();
        for _ in 0..50 {
            open.clear();
            let mut heap = BinaryHeap::new();
            for _ in 0..1 + draw(12) {
                let (f, node) = (1_000 + draw(4 * OPEN_SPAN), draw(64));
                open.push(f, node as usize);
                heap.push(Reverse((f, node)));
            }
            for _ in 0..400 {
                let Some(Reverse((f, node))) = heap.pop() else {
                    break;
                };
                assert_eq!(open.pop(), Some((f, node as usize)));
                for _ in 0..draw(4) {
                    let (step, next) = (f + draw(2 * DIAGONAL + 1), draw(64));
                    open.push(step, next as usize);
                    heap.push(Reverse((step, next)));
                }
            }
        }
    }
}
