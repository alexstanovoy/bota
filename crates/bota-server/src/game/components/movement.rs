//! What an entity keeps while it is walking somewhere.

use bota_proto::Vec2;

/// The route a player-driven entity is walking. Absent when it has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// Corners still ahead, the next one first.
    pub path: Vec<Vec2>,
    /// Where the route was laid to.
    pub goal: Vec2,
    /// The way round a body it settled on, while it is still working round.
    pub trace: Option<TraceSide>,
}

/// Which way round a body a creep decided to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceSide {
    /// To the left of the line to the body.
    Left,
    /// To the right of it.
    Right,
}

/// What a creep keeps while marching its lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct March {
    /// How far along its route it has got, as a waypoint number.
    pub route_step: u16,
    /// The way round it settled on, while it is still working round.
    pub trace: Option<TraceSide>,
    /// Ticks running that it wanted to move and could not.
    pub shove: u32,
}
