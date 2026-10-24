//! What an entity keeps while it is walking somewhere.

use bota_proto::{Fixed, Vec2};

use crate::engine::Entity;

/// The route an entity walks: the corners of a way found round what stands
/// still, to the goal it was laid for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// Corners still ahead, the next one first, the end last. Empty for a
    /// straight walk at the end, or with no route laid.
    pub corners: Vec<Vec2>,
    /// Where the route was laid to. Absent with no route laid.
    pub goal: Option<Vec2>,
    /// Where the route ends: the goal when it can be stood on and reached,
    /// else the nearest spot that can.
    pub end: Vec2,
    /// Whether the walk has come as near its goal as it gets: it stands
    /// until the goal moves.
    pub done: bool,
    /// The stationary bodies the route was laid round, as circles where
    /// they stand: the short pather's trace round what its long path runs
    /// into, kept while those bodies stand there. Empty for a route laid
    /// over the ground alone.
    pub round: Vec<(Vec2, Fixed)>,
}

impl Route {
    pub fn none() -> Route {
        Route {
            corners: Vec::new(),
            goal: None,
            end: Vec2::ZERO,
            done: false,
            round: Vec::new(),
        }
    }
}

/// What a creep keeps while marching its lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct March {
    /// The waypoint of its lane route it aims at next.
    pub next: u16,
}

/// How an entity has been moving.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Motion {
    /// How far it moved last tick.
    pub delta: Vec2,
    /// Ticks running that it wanted to move and could not.
    pub stalled: u32,
    /// Ticks running that it has not moved.
    pub still: u32,
    /// The body that blocks its walk, while it waits on it or lays its way
    /// round it (`rules::UNIT_BLOCKED_WAIT_TICKS`).
    pub blocked: Option<Blocked>,
    /// The wall it traces round, while its short pather finds no way on.
    pub trace: Option<Trace>,
}

/// A walker tracing round what shuts it in, the short pather's
/// wall-tracing: it keeps the obstacle on one hand until it stands nearer
/// its goal than where it met it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trace {
    /// The way it turns off its aim to find a free step: 1 to its left, -1
    /// to its right.
    pub side: i8,
    /// The spot it walks to.
    pub goal: Vec2,
    /// Where it met the wall.
    pub hit: Vec2,
    /// Whether it has walked off the spot it met the wall at.
    pub left_hit: bool,
}

/// A walker's wait on the body that blocks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blocked {
    /// The body in its way.
    pub by: Entity,
    /// The tick from which it lays its way round that body.
    pub until: u32,
    /// Which of the waits it is on: 0, 1 or 2.
    pub tier: u8,
}

/// The next stretch of an entity's walk, tick by tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Where it stands after each tick of the plan, in order, the first of
    /// them after the tick the plan was laid on.
    pub steps: Vec<Vec2>,
    /// The tick the first step is taken on.
    pub from: u32,
    /// The next step to take, as an index into `steps`. Their number once
    /// the plan is walked.
    pub at: usize,
    /// The route goal the plan was laid for.
    pub goal: Vec2,
    /// The step per tick it was laid for.
    pub step: Fixed,
    /// The tick it was laid on.
    pub laid: u32,
    /// Whether it was laid to the route's end rather than a spot on the
    /// way: it is walked to its last step however few are left.
    pub last: bool,
}

impl Plan {
    pub fn none() -> Plan {
        Plan {
            steps: Vec::new(),
            from: 0,
            at: 0,
            goal: Vec2::ZERO,
            step: Fixed::ZERO,
            laid: 0,
            last: false,
        }
    }

    /// Whether the plan has steps left to take.
    pub fn stands(&self) -> bool {
        self.at < self.steps.len()
    }

    /// How many ticks of the plan are left to walk.
    pub fn left(&self) -> usize {
        self.steps.len().saturating_sub(self.at)
    }

    pub fn clear(&mut self) {
        self.steps.clear();
        self.at = 0;
    }
}
