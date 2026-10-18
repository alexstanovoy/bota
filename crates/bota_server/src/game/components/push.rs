use bota_proto::{Fixed, Vec2};

use crate::engine::Entity;

/// A unit being carried to a spot, as a gust carries it.
///
/// It sits on the unit carried, which cannot walk meanwhile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Push {
    /// Who sent it. The handle may outlive the body.
    pub source: Entity,
    /// The spot it is carried to.
    pub aim: Vec2,
    /// World units a second.
    pub speed: Fixed,
    /// The last spot on its way where a body may stand.
    pub clear: Vec2,
}
