//! The next stretch of a walk: A* over where a body can be and when, round
//! what stands still and what is about to move.
//!
//! A state is a spot, the way the body faces and the tick it is there. A
//! step of the search is a straight stretch along one of sixteen headings
//! or straight at the goal, the turn onto it paid for first in ticks stood
//! still, or a stretch stood waiting, or the last stretch straight at the
//! goal tick by tick. The cost is time.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bota_proto::{Angle, Fixed, Vec2};
use rustc_hash::FxHashMap;

use crate::game::{
    Obstacles, facing_gap, facing_towards, heading_of, isqrt64, move_towards, point_along, rules,
    stall_ticks,
};

/// How many headings a stretch may be walked along.
const HEADINGS: u32 = 16;
/// Brads between two neighbouring headings.
const HEADING_BRADS: u32 = 65536 / HEADINGS;
/// The heading number a facing that is none of the sixteen is told apart by.
const ANY_HEADING: u32 = HEADINGS;
/// How far off the way to the goal a heading may point and still be
/// tried, in brads: five of the sixteen either way.
const SIDEWAYS_BRADS: u32 = 5 * HEADING_BRADS;
/// Ticks one stretch lasts.
const PRIM: u32 = rules::LOCAL_PRIM_TICKS;
/// Ticks a plan reaches, at most.
const HORIZON: u32 = rules::LOCAL_HORIZON_TICKS;

/// A body near the walker and where it will be.
#[derive(Clone, Copy, Debug)]
pub struct Foreseen<'a> {
    /// Where it stands now.
    pub at: Vec2,
    /// Its collision size.
    pub radius: Fixed,
    /// How far it moved last tick.
    pub delta: Vec2,
    /// Where its own plan has it after each tick from `from` on. Empty for
    /// a body with no plan to read.
    pub steps: &'a [Vec2],
    /// The tick the first of its planned steps is taken on.
    pub from: u32,
}

impl Foreseen<'_> {
    /// Where the body is after a tick: by its plan while that reaches, else
    /// its last step carried forward for [`rules::PREDICT_TICKS`] and held
    /// there.
    pub fn at_tick(&self, now: u32, tick: u32) -> Vec2 {
        if !self.steps.is_empty() && tick >= self.from {
            let index = (tick - self.from) as usize;
            return self.steps[index.min(self.steps.len() - 1)];
        }
        let ahead = i64::from(tick.saturating_sub(now).min(rules::PREDICT_TICKS));
        Vec2 {
            x: Fixed {
                raw: (i64::from(self.at.x.raw) + i64::from(self.delta.x.raw) * ahead)
                    .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            },
            y: Fixed {
                raw: (i64::from(self.at.y.raw) + i64::from(self.delta.y.raw) * ahead)
                    .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            },
        }
    }

    /// Whether the body is going anywhere.
    fn moves(&self) -> bool {
        self.delta != Vec2::ZERO || !self.steps.is_empty()
    }

    /// How far from where it stands the body gets over the horizon, in raw
    /// units.
    fn reach(&self, now: u32) -> i64 {
        if self.steps.is_empty() {
            return isqrt64(self.delta.length_squared()) * i64::from(rules::PREDICT_TICKS);
        }
        let mut far = 0;
        for tick in now + 1..=now + HORIZON {
            far = far.max(self.at.distance_squared(self.at_tick(now, tick)));
        }
        isqrt64(far)
    }
}

/// What a walk asks of the search.
#[derive(Clone, Copy, Debug)]
pub struct LocalAsk {
    /// Where the body stands.
    pub from: Vec2,
    /// The way it faces.
    pub facing: Angle,
    /// Where it is going.
    pub goal: Vec2,
    /// How near the goal is near enough.
    pub arrive: Fixed,
    /// Its collision size.
    pub radius: Fixed,
    /// How far it walks in a tick.
    pub step: Fixed,
    /// Brads it turns in a tick.
    pub turn_rate: u16,
    /// The tick the plan is laid on: its first step is taken on the next.
    pub now: u32,
}

/// One state of the search.
#[derive(Clone, Copy, Debug)]
struct Node {
    /// Where the body is.
    pos: Vec2,
    /// The way it faces there.
    facing: Angle,
    /// Which of the headings it faces, or [`ANY_HEADING`].
    heading: u32,
    /// Ticks from the start it is there at.
    t: u32,
    /// Turns taken on the way.
    turns: u32,
    /// The state it came from. `u32::MAX` for the start.
    parent: u32,
    /// Where the steps that got it here begin in the trail, and how many.
    trail: (u32, u32),
    /// How far the goal is, in raw units, less how near is near enough.
    far: i64,
    /// Ticks the goal is still away at best.
    left: u32,
}

/// The scratch a local search works in, kept between searches.
#[derive(Default)]
pub struct LocalScratch {
    /// Every state made.
    nodes: Vec<Node>,
    /// The steps of every state, end to end.
    trail: Vec<Vec2>,
    /// The open list: cost plus the estimate left, how far the goal is,
    /// turns taken, the state.
    heap: BinaryHeap<Reverse<(u32, i64, u32, u32)>>,
    /// The best state at each spot, heading and stretch of time.
    ///
    /// A hash map, not an ordered one: the search only looks states up, and
    /// clearing it between plans keeps its allocation for the next search.
    seen: FxHashMap<u64, u32>,
    /// The squared distance each body must be kept at, in the order of the
    /// bodies asked about.
    need: Vec<i64>,
    /// How far each body may stray from where it stands, in raw units,
    /// and the distance it must be kept at.
    reach: Vec<i64>,
    /// Whether any body asked about is going anywhere.
    moving: bool,
    /// Where each body asked about stands after each tick of the horizon, a
    /// row of [`HORIZON`] per body in the order asked about.
    foreseen: Vec<Vec2>,
    /// The offset of each tick of a stretch along each of the headings,
    /// heading by heading.
    offsets: Vec<Vec2>,
    /// The step per tick the offsets are laid for.
    offsets_step: Option<i32>,
    /// The steps of the stretch being tried.
    steps: Vec<Vec2>,
}

impl LocalScratch {
    /// Scratch that has searched nothing.
    pub fn new() -> LocalScratch {
        LocalScratch::default()
    }
}

/// Where the body stands after each tick of the best stretch found, and
/// whether that gets it to the goal: there when a state gets there within
/// the horizon and the search budget, else as near as any state got. Empty
/// when nothing gets it anywhere, or it is there already.
pub fn plan_local(
    ob: &Obstacles,
    bodies: &[Foreseen],
    ask: &LocalAsk,
    scratch: &mut LocalScratch,
) -> (Vec<Vec2>, bool) {
    if ask.step.raw <= 0 || ask.from.within(ask.goal, ask.arrive) {
        return (Vec::new(), true);
    }
    scratch.nodes.clear();
    scratch.trail.clear();
    scratch.heap.clear();
    scratch.seen.clear();
    scratch.need.clear();
    scratch.reach.clear();
    scratch.moving = false;
    scratch.foreseen.clear();
    for body in bodies {
        let apart = ask.from.distance_squared(body.at);
        let need = (ask.radius + body.radius).squared_raw();
        // A body it already overlaps stops only a step deeper into it.
        let need = need.min(apart);
        scratch.need.push(need);
        // Where the body stands after each tick of the horizon: the same
        // answer for every stretch tried, so it is worked out once.
        for step in 1..=HORIZON {
            scratch.foreseen.push(body.at_tick(ask.now, ask.now + step));
        }
        scratch.reach.push(body.reach(ask.now) + isqrt64(need));
        scratch.moving |= body.moves();
    }
    debug_assert_eq!(scratch.foreseen.len(), bodies.len() * HORIZON as usize);
    if scratch.offsets_step != Some(ask.step.raw) {
        scratch.offsets.clear();
        for heading in 0..HEADINGS {
            let angle = Angle {
                brads: (heading * HEADING_BRADS) as u16,
            };
            let towards = heading_of(angle);
            for k in 1..=PRIM {
                scratch.offsets.push(point_along(
                    Vec2::ZERO,
                    towards,
                    Fixed {
                        raw: ask.step.raw.saturating_mul(k as i32),
                    },
                ));
            }
        }
        debug_assert_eq!(scratch.offsets.len(), HEADINGS as usize * PRIM as usize);
        scratch.offsets_step = Some(ask.step.raw);
    }
    let (far, left) = goal_gap(ask, ask.from);
    let start = Node {
        pos: ask.from,
        facing: ask.facing,
        heading: heading_number(ask.facing),
        t: 0,
        turns: 0,
        parent: u32::MAX,
        trail: (0, 0),
        far,
        left,
    };
    scratch.nodes.push(start);
    scratch.seen.insert(key_of(&start), 0);
    scratch.heap.push(Reverse((start.left, start.far, 0, 0)));
    let mut best = (start.far, 0u32, 0u32);
    let mut expanded = 0;
    while let Some(Reverse((_, _, _, index))) = scratch.heap.pop() {
        let node = scratch.nodes[index as usize];
        if scratch.seen.get(&key_of(&node)) != Some(&index) {
            continue; // a way to the same state that was beaten since
        }
        if node.pos.within(ask.goal, ask.arrive) {
            return (walk_back(scratch, index), true);
        }
        if expanded >= rules::LOCAL_EXPANSIONS {
            break;
        }
        expanded += 1;
        if (node.far, node.t) < (best.0, best.1) {
            best = (node.far, node.t, index);
        }
        try_straight_at_goal(ob, bodies, ask, scratch, index);
        let at_goal = facing_towards(node.pos, ask.goal);
        if heading_number(at_goal) == ANY_HEADING {
            try_heading(ob, bodies, ask, scratch, index, at_goal, ANY_HEADING);
        }
        // Headings that lead away from the goal are not tried: a way back
        // is the route's to find, not the plan's.
        for heading in 0..HEADINGS {
            let angle = Angle {
                brads: (heading * HEADING_BRADS) as u16,
            };
            if u32::from(facing_gap(angle, at_goal)) > SIDEWAYS_BRADS {
                continue;
            }
            try_heading(ob, bodies, ask, scratch, index, angle, heading);
        }
        // Waiting is for letting a body pass: with nothing moving there is
        // nothing to wait for.
        if scratch.moving {
            try_waiting(bodies, ask, scratch, index);
        }
    }
    // Short of the goal, a state is worth walking to only when it is
    // nearer the goal by a couple of steps: less is standing about.
    let gained = start.far - best.0;
    if best.2 == 0 || gained < i64::from(ask.step.raw) * 2 {
        return (Vec::new(), false);
    }
    (walk_back(scratch, best.2), false)
}

/// The last stretch: straight at the goal, tick by tick, until near enough,
/// when that takes no more than two stretches' worth of ticks.
fn try_straight_at_goal(
    ob: &Obstacles,
    bodies: &[Foreseen],
    ask: &LocalAsk,
    scratch: &mut LocalScratch,
    index: u32,
) {
    let node = scratch.nodes[index as usize];
    let most = i64::from(ask.step.raw) * i64::from(2 * PRIM);
    if node.far > most {
        return;
    }
    let mut steps = std::mem::take(&mut scratch.steps);
    steps.clear();
    let mut at = node.pos;
    let mut there = false;
    for _ in 0..2 * PRIM {
        at = move_towards(at, ask.goal, ask.step);
        steps.push(at);
        if at.within(ask.goal, ask.arrive) {
            there = true;
            break;
        }
    }
    if there {
        let angle = facing_towards(node.pos, at);
        let stall = stall_ticks(node.facing, angle, ask.turn_rate);
        let ticks = stall + steps.len() as u32;
        if node.t + ticks <= HORIZON
            && worth(scratch, at, ANY_HEADING, node.t + ticks)
            && ob.clear(node.pos, at, ask.radius)
        {
            let turned = facing_gap(node.facing, angle) > 0;
            add_stretch(
                bodies,
                ask,
                scratch,
                index,
                angle,
                ANY_HEADING,
                stall,
                &steps,
                turned,
            );
        }
    }
    scratch.steps = steps;
}

/// One stretch along a heading, the turn onto it stood through first: one
/// of the sixteen, or the way straight at the goal.
fn try_heading(
    ob: &Obstacles,
    bodies: &[Foreseen],
    ask: &LocalAsk,
    scratch: &mut LocalScratch,
    index: u32,
    angle: Angle,
    heading: u32,
) {
    let node = scratch.nodes[index as usize];
    let stall = stall_ticks(node.facing, angle, ask.turn_rate);
    let ticks = stall + PRIM;
    if node.t + ticks > HORIZON {
        return;
    }
    let mut steps = std::mem::take(&mut scratch.steps);
    steps.clear();
    if heading < HEADINGS {
        let from = (heading * PRIM) as usize;
        for offset in &scratch.offsets[from..from + PRIM as usize] {
            steps.push(node.pos + *offset);
        }
    } else {
        let towards = node.pos + heading_of(angle);
        for k in 1..=PRIM {
            steps.push(point_along(
                node.pos,
                towards,
                Fixed {
                    raw: ask.step.raw.saturating_mul(k as i32),
                },
            ));
        }
    }
    let end = *steps.last().expect("a stretch has steps");
    if worth(scratch, end, heading, node.t + ticks) && ob.clear(node.pos, end, ask.radius) {
        let turned = facing_gap(node.facing, angle) > 0;
        add_stretch(
            bodies, ask, scratch, index, angle, heading, stall, &steps, turned,
        );
    }
    scratch.steps = steps;
}

/// One stretch stood still, for a body to pass.
fn try_waiting(bodies: &[Foreseen], ask: &LocalAsk, scratch: &mut LocalScratch, index: u32) {
    let node = scratch.nodes[index as usize];
    if node.t + PRIM > HORIZON || !worth(scratch, node.pos, node.heading, node.t + PRIM) {
        return;
    }
    let mut steps = std::mem::take(&mut scratch.steps);
    steps.clear();
    steps.resize(PRIM as usize, node.pos);
    add_stretch(
        bodies,
        ask,
        scratch,
        index,
        node.facing,
        node.heading,
        0,
        &steps,
        false,
    );
    scratch.steps = steps;
}

/// Whether a state at a spot, heading and tick is one not got to sooner
/// already.
fn worth(scratch: &LocalScratch, pos: Vec2, heading: u32, t: u32) -> bool {
    let key = key_parts(pos, heading, t);
    scratch
        .seen
        .get(&key)
        .is_none_or(|&had| scratch.nodes[had as usize].t > t)
}

/// Adds the state a stretch ends in, if no body is met on the way.
#[expect(clippy::too_many_arguments, reason = "one stretch has this many parts")]
fn add_stretch(
    bodies: &[Foreseen],
    ask: &LocalAsk,
    scratch: &mut LocalScratch,
    index: u32,
    facing: Angle,
    heading: u32,
    stall: u32,
    steps: &[Vec2],
    turned: bool,
) {
    let node = scratch.nodes[index as usize];
    let end = *steps.last().expect("a stretch has steps");
    let ticks = stall + steps.len() as u32;
    debug_assert!(node.t + ticks <= HORIZON);
    let far = isqrt64(node.pos.distance_squared(end));
    for (b, body) in bodies.iter().enumerate() {
        let need = scratch.need[b];
        // A body that cannot come near the stretch is not looked at tick
        // by tick.
        let span = far + scratch.reach[b];
        if node.pos.distance_squared(body.at) > span * span {
            continue;
        }
        let row = b * HORIZON as usize;
        for i in 0..ticks {
            let mine = if i < stall {
                node.pos
            } else {
                steps[(i - stall) as usize]
            };
            if mine.distance_squared(scratch.foreseen[row + node.t as usize + i as usize]) < need {
                return;
            }
        }
    }
    let (goal_far, left) = goal_gap(ask, end);
    let next = Node {
        pos: end,
        facing,
        heading,
        t: node.t + ticks,
        turns: node.turns + u32::from(turned),
        parent: index,
        trail: (scratch.trail.len() as u32, ticks),
        far: goal_far,
        left,
    };
    let at = scratch.nodes.len() as u32;
    for _ in 0..stall {
        scratch.trail.push(node.pos);
    }
    scratch.trail.extend_from_slice(steps);
    scratch.nodes.push(next);
    scratch.seen.insert(key_of(&next), at);
    scratch
        .heap
        .push(Reverse((next.t + next.left, next.far, next.turns, at)));
}

/// The steps from the start to a state, in order.
fn walk_back(scratch: &LocalScratch, index: u32) -> Vec<Vec2> {
    let mut stretches = Vec::new();
    let mut at = index;
    while at != u32::MAX {
        let node = &scratch.nodes[at as usize];
        if node.trail.1 > 0 {
            stretches.push(node.trail);
        }
        at = node.parent;
    }
    let mut steps = Vec::new();
    for (from, len) in stretches.into_iter().rev() {
        steps.extend_from_slice(&scratch.trail[from as usize..(from + len) as usize]);
    }
    steps
}

/// How far the goal is from a spot, less how near is near enough, in raw
/// units and never below zero, and the ticks that is at the least.
fn goal_gap(ask: &LocalAsk, at: Vec2) -> (i64, u32) {
    let far = (isqrt64(at.distance_squared(ask.goal)) - i64::from(ask.arrive.raw)).max(0);
    let step = i64::from(ask.step.raw);
    (far, ((far + step - 1) / step) as u32)
}

/// Which of the headings a facing is, or [`ANY_HEADING`] for none of them.
fn heading_number(facing: Angle) -> u32 {
    let brads = u32::from(facing.brads);
    if brads.is_multiple_of(HEADING_BRADS) {
        brads / HEADING_BRADS
    } else {
        ANY_HEADING
    }
}

/// The key two states are told apart by: the spot to
/// [`rules::LOCAL_KEY_CELL`], the heading and the stretch of time.
fn key_of(node: &Node) -> u64 {
    key_parts(node.pos, node.heading, node.t)
}

/// The key of a state at a spot, heading and tick: neighbouring headings
/// share a key, so a wall of bodies is not felt along at every angle.
fn key_parts(pos: Vec2, heading: u32, t: u32) -> u64 {
    let cell = |v: Fixed| (v.to_int().max(0) / rules::LOCAL_KEY_CELL) as u64;
    (cell(pos.x) << 40) | (cell(pos.y) << 20) | (u64::from(heading / 2) << 8) | u64::from(t / PRIM)
}
