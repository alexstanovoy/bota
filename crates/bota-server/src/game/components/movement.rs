//! What an entity keeps while it is walking somewhere.

use bota_proto::{Fixed, Vec2};

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
}

impl Route {
    /// No route laid.
    pub fn none() -> Route {
        Route {
            corners: Vec::new(),
            goal: None,
            end: Vec2::ZERO,
            done: false,
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
    /// The tick it stands until after walking into a body that was itself
    /// moving. Zero when it waits on nothing.
    pub wait_until: u32,
    /// The tick its route was last laid round the bodies standing about
    /// it. Zero when it never was.
    pub relaid: u32,
    /// The tick it last ran into a hero. Zero when it never did.
    pub bumped: u32,
    /// How many times running it has run into a hero within
    /// [`rules::HEED_HERO_TICKS`] of the time before.
    ///
    /// [`rules::HEED_HERO_TICKS`]: crate::game::rules::HEED_HERO_TICKS
    pub bumps: u32,
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
    /// No plan laid.
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

    /// Where the plan has the entity after a tick: the step for that tick,
    /// the last step past the plan's end, and nothing before its start.
    pub fn at_tick(&self, tick: u32) -> Option<Vec2> {
        if self.steps.is_empty() || tick < self.from {
            return None;
        }
        let index = (tick - self.from) as usize;
        Some(self.steps[index.min(self.steps.len() - 1)])
    }

    /// Forgets the plan.
    pub fn clear(&mut self) {
        self.steps.clear();
        self.at = 0;
    }
}
