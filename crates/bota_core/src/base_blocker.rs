//! Team-only base exits placed by the full map.
use bota_proto::{Angle, Fixed, Team, Vec2};

use crate::point_ahead;

/// Both day and night sight of a base exit (7.40).
pub const BASE_BLOCKER_VISION: i32 = 700;
/// Each field consists of fifteen overlapping circular bodies.
pub const BASE_BLOCKER_BODIES: i32 = 15;
/// Each body has this radius; their centres are this far apart.
pub const BASE_BLOCKER_RADIUS: i32 = 25;
/// The enclosing radius of the whole row, for broad-phase queries.
pub const BASE_BLOCKER_BOUND: i32 = BASE_BLOCKER_RADIUS * (BASE_BLOCKER_BODIES / 2 + 1);

/// A permanent, non-targetable map field that permits its own side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaseBlockerDef {
    pub at: Vec2,
    pub facing: Angle,
    pub team: Team,
    /// Source map elevation, independent of the walkability grid's height tiers.
    pub height: Fixed,
}

impl BaseBlockerDef {
    /// The circular bodies of the field, in their placement order.
    pub fn bodies(&self) -> impl Iterator<Item = (Vec2, Fixed)> + '_ {
        (0..BASE_BLOCKER_BODIES).map(|index| {
            let distance = Fixed::from_int((BASE_BLOCKER_BODIES / 2 - index) * BASE_BLOCKER_RADIUS);
            (
                point_ahead(self.at, self.facing, distance),
                Fixed::from_int(BASE_BLOCKER_RADIUS),
            )
        })
    }
}

// The four permanent fields and their orientation, established from installed
// game build 6975.
pub const DOTA_BASE_BLOCKERS: [BaseBlockerDef; 4] = [
    blocker(152340, 628660, 0, Team::Radiant, 27357),
    blocker(1657600, 1164800, 0, Team::Dire, 27648),
    blocker(577102, 174088, 49152, Team::Radiant, 27357),
    blocker(1216000, 1651200, 49152, Team::Dire, 27648),
];

const fn blocker(x: i32, y: i32, brads: u16, team: Team, height: i32) -> BaseBlockerDef {
    BaseBlockerDef {
        at: Vec2 {
            x: Fixed::from_ratio(x, 100),
            y: Fixed::from_ratio(y, 100),
        },
        facing: Angle { brads },
        team,
        height: Fixed::from_ratio(height, 100),
    }
}
