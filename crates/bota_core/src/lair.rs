//! A neutral boss's homes and lifecycle, independent of a particular map.
use crate::NeutralKind;
use bota_proto::{ObjectiveKind, Vec2};

#[derive(Clone, Copy, Debug)]
pub struct LairDef {
    pub objective: ObjectiveKind,
    pub neutral: NeutralKind,
    pub day: Vec2,
    pub night: Vec2,
    /// Seconds after the horn before the natural day/night cycle chooses the home.
    pub move_after: u32,
    /// Delay after a death, inclusive, in seconds.
    pub respawn: [u32; 2],
}
