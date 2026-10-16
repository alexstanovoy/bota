//! Where an entity stands and how much room it takes.

use bota_proto::{Angle, Fixed, Vec2};

/// Where an entity is and which way it looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transform {
    /// Position on the map, in world units.
    pub pos: Vec2,
    /// The way it faces. Turning takes time, so this does not follow from the
    /// way it is walking.
    pub facing: Angle,
}

/// The two circles an entity's body is: the one nothing walks into, and
/// the one its reach and everything reaching it are measured to. Absent for
/// whatever has no body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hull {
    /// Collision size: how near another body's centre may come, less that
    /// body's own, in world units.
    pub collision: Fixed,
    /// Bound radius: where its edge is for attack range, cast range and
    /// areas, in world units.
    pub bound: Fixed,
}
