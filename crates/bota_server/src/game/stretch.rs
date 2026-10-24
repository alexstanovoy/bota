//! The next stretch of a walk, Dota's short pather (Liquipedia,
//! *Pathfinding*): A* over where a body can be and when, round the ground and
//! the stationary bodies the walk is asked to keep off, which stand where
//! they are for the whole plan. Moving bodies are none of its business.
//!
//! A state is a spot, the way the body faces and the tick it is there. A
//! step of the search is a straight stretch along one of sixteen headings
//! or straight at the goal, the turn onto it paid for first in ticks stood
//! still, or the last stretch straight at the goal tick by tick. The cost is
//! time.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bota_core::{
    facing_gap, facing_towards, heading_of, isqrt64, move_towards, point_along, rules, stall_ticks,
};
use bota_proto::{Angle, Fixed, Vec2};
use rustc_hash::FxHashMap;

use crate::game::Obstacles;

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
#[derive(Clone, Default)]
pub struct LocalScratch {
    /// Every state made.
    nodes: Vec<Node>,
    /// The steps of every state, end to end.
    trail: Vec<Vec2>,
    /// The open list: cost plus the estimate left, how far the goal is,
    /// turns taken, the state.
    heap: BinaryHeap<Reverse<(u32, i64, u32, u32)>>,
    /// The best state at each spot, heading and stretch of time.
    seen: FxHashMap<u64, u32>,
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

    /// Forgets the last search and works out what every stretch tried in
    /// the next one reads: the offsets of every heading.
    fn prepare(&mut self, ask: &LocalAsk) {
        self.nodes.clear();
        self.trail.clear();
        self.heap.clear();
        self.seen.clear();
        if self.offsets_step != Some(ask.step.raw) {
            self.offsets.clear();
            for heading in 0..HEADINGS {
                let angle = Angle {
                    brads: (heading * HEADING_BRADS) as u16,
                };
                let towards = heading_of(angle);
                for k in 1..=PRIM {
                    self.offsets.push(point_along(
                        Vec2::ZERO,
                        towards,
                        Fixed {
                            raw: ask.step.raw.saturating_mul(k as i32),
                        },
                    ));
                }
            }
            debug_assert_eq!(self.offsets.len(), HEADINGS as usize * PRIM as usize);
            self.offsets_step = Some(ask.step.raw);
        }
    }
}

/// Where the body stands after each tick of the best stretch found, and
/// whether that gets it to the goal: there when a state gets there within
/// the horizon and the search budget, else as near as any state got. Empty
/// when nothing gets it anywhere; empty and there when it is there already
/// or cannot step.
pub fn plan_local(ob: &Obstacles, ask: &LocalAsk, scratch: &mut LocalScratch) -> (Vec<Vec2>, bool) {
    if ask.step.raw <= 0 || ask.from.within(ask.goal, ask.arrive) {
        return (Vec::new(), true);
    }
    // With the straight way to the goal clear, that is the shortest stretch:
    // the turn onto it stood through, then step by step at it.
    if ob.clear(ask.from, ask.goal, ask.radius) {
        return straight_at_goal(ask);
    }
    scratch.prepare(ask);
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
        expand(ob, ask, scratch, index, node.pos);
    }
    // Short of the goal, a state is walked to only when it is at least two
    // steps nearer the goal than the start.
    let gained = start.far - best.0;
    if best.2 == 0 || gained < i64::from(ask.step.raw) * 2 {
        return (Vec::new(), false);
    }
    (walk_back(scratch, best.2), false)
}

/// The plan of a walk straight at the goal over a clear way: the ticks the
/// turn onto it takes stood where the body is, then a step a tick until near
/// enough, at most the horizon.
fn straight_at_goal(ask: &LocalAsk) -> (Vec<Vec2>, bool) {
    let turn = stall_ticks(
        ask.facing,
        facing_towards(ask.from, ask.goal),
        ask.turn_rate,
    );
    let mut steps = vec![ask.from; turn.min(HORIZON) as usize];
    let mut at = ask.from;
    while (steps.len() as u32) < HORIZON && !at.within(ask.goal, ask.arrive) {
        at = move_towards(at, ask.goal, ask.step);
        steps.push(at);
    }
    let reached = at.within(ask.goal, ask.arrive);
    (steps, reached)
}

/// Tries every stretch out of one state: straight at the goal, and along each
/// heading within [`SIDEWAYS_BRADS`] of the goal.
fn expand(ob: &Obstacles, ask: &LocalAsk, scratch: &mut LocalScratch, index: u32, pos: Vec2) {
    try_straight_at_goal(ob, ask, scratch, index);
    let at_goal = facing_towards(pos, ask.goal);
    if heading_number(at_goal) == ANY_HEADING {
        try_heading(ob, ask, scratch, index, at_goal, ANY_HEADING);
    }
    for heading in 0..HEADINGS {
        let angle = Angle {
            brads: (heading * HEADING_BRADS) as u16,
        };
        if u32::from(facing_gap(angle, at_goal)) > SIDEWAYS_BRADS {
            continue;
        }
        try_heading(ob, ask, scratch, index, angle, heading);
    }
}

/// The last stretch: straight at the goal, tick by tick, until near enough,
/// when that takes no more than two stretches' worth of ticks.
fn try_straight_at_goal(ob: &Obstacles, ask: &LocalAsk, scratch: &mut LocalScratch, index: u32) {
    let node = scratch.nodes[index as usize];
    let most = i64::from(ask.step.raw) * i64::from(2 * PRIM);
    if node.far > most {
        return;
    }
    scratch.steps.clear();
    let mut at = node.pos;
    for _ in 0..2 * PRIM {
        at = move_towards(at, ask.goal, ask.step);
        scratch.steps.push(at);
        if at.within(ask.goal, ask.arrive) {
            break;
        }
    }
    if !at.within(ask.goal, ask.arrive) {
        return;
    }
    let angle = facing_towards(node.pos, at);
    let stall = stall_ticks(node.facing, angle, ask.turn_rate);
    let ticks = stall + scratch.steps.len() as u32;
    if node.t + ticks <= HORIZON
        && worth(scratch, at, ANY_HEADING, node.t + ticks)
        && ob.clear(node.pos, at, ask.radius)
    {
        let turned = facing_gap(node.facing, angle) > 0;
        add_stretch(ask, scratch, index, angle, ANY_HEADING, stall, turned);
    }
}

/// One stretch along a heading, the turn onto it stood through first: one
/// of the sixteen, or the way straight at the goal.
fn try_heading(
    ob: &Obstacles,
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
    let towards = node.pos + heading_of(angle);
    let step_at = |k: u32| -> Vec2 {
        if heading < HEADINGS {
            node.pos + scratch.offsets[(heading * PRIM + k - 1) as usize]
        } else {
            point_along(
                node.pos,
                towards,
                Fixed {
                    raw: ask.step.raw.saturating_mul(k as i32),
                },
            )
        }
    };
    let end = step_at(PRIM);
    if !worth(scratch, end, heading, node.t + ticks) || !ob.clear(node.pos, end, ask.radius) {
        return;
    }
    scratch.steps.clear();
    scratch.steps.extend((1..=PRIM).map(step_at));
    let turned = facing_gap(node.facing, angle) > 0;
    add_stretch(ask, scratch, index, angle, heading, stall, turned);
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

/// Adds the state the stretch in `scratch.steps` ends in.
fn add_stretch(
    ask: &LocalAsk,
    scratch: &mut LocalScratch,
    index: u32,
    facing: Angle,
    heading: u32,
    stall: u32,
    turned: bool,
) {
    let node = scratch.nodes[index as usize];
    let end = *scratch.steps.last().expect("a stretch has steps");
    let ticks = stall + scratch.steps.len() as u32;
    debug_assert!(node.t + ticks <= HORIZON);
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
    scratch.trail.extend_from_slice(&scratch.steps);
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

/// The key of a state at a spot, heading and tick: each pair of
/// neighbouring headings shares a key.
fn key_parts(pos: Vec2, heading: u32, t: u32) -> u64 {
    let cell = |v: Fixed| (v.to_int().max(0) / rules::LOCAL_KEY_CELL) as u64;
    (cell(pos.x) << 40) | (cell(pos.y) << 20) | (u64::from(heading / 2) << 8) | u64::from(t / PRIM)
}
