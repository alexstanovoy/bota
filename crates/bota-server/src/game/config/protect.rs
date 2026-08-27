//! Which structures may be struck, and what has to fall first.
//!
//! A map names its rules as data: each guards a structure with a condition
//! over the same side's already-fallen structures. A structure no rule guards
//! may be struck from the first tick.

use crate::game::rules;

/// Names structures of one side on the map.
///
/// One name may fit several bodies — the two tier fours share a lane and a
/// tier — and then it names all of them at once: such a group counts as
/// fallen only when none of it is left standing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructureId {
    /// The towers of a lane and tier.
    Tower {
        /// Which lane.
        lane: u8,
        /// Which tier.
        tier: u8,
    },
    /// The barracks of a lane, melee or ranged.
    Barracks {
        /// Which lane.
        lane: u8,
        /// Whether it is the one feeding the ranged creeps.
        ranged: bool,
    },
    /// The Ancient.
    Ancient,
}

/// A condition over what has already fallen on one side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Until {
    /// Nothing named by this id is left standing.
    Down(StructureId),
    /// At least one of the conditions holds.
    AnyOf(&'static [Until]),
    /// Every condition holds.
    AllOf(&'static [Until]),
}

/// One rule: what it guards, and what must fall before that may be struck.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Protection {
    /// Which structures the rule guards.
    pub guarded: StructureId,
    /// The condition that lifts the guard.
    pub until: Until,
}

const fn tower(lane: u8, tier: u8) -> StructureId {
    StructureId::Tower { lane, tier }
}

const fn rax(lane: u8, ranged: bool) -> StructureId {
    StructureId::Barracks { lane, ranged }
}

const fn chain(lane: u8) -> [Protection; 4] {
    [
        Protection {
            guarded: tower(lane, 2),
            until: Until::Down(tower(lane, 1)),
        },
        Protection {
            guarded: tower(lane, 3),
            until: Until::Down(tower(lane, 2)),
        },
        Protection {
            guarded: rax(lane, false),
            until: Until::Down(tower(lane, 3)),
        },
        Protection {
            guarded: rax(lane, true),
            until: Until::Down(tower(lane, 3)),
        },
    ]
}

const MID: [Protection; 4] = chain(rules::LANE_MID);
const TOP: [Protection; 4] = chain(rules::LANE_TOP);
const BOT: [Protection; 4] = chain(rules::LANE_BOT);

/// Any tier three fallen opens the pair of tier fours.
const ANY_TIER_THREE: [Until; 3] = [
    Until::Down(tower(rules::LANE_MID, 3)),
    Until::Down(tower(rules::LANE_TOP, 3)),
    Until::Down(tower(rules::LANE_BOT, 3)),
];

/// The Dota rules: each lane opens tower by tower into its barracks, the
/// tier fours wait on a broken lane, and the Ancient waits on both of them.
pub const DOTA_PROTECTION: [Protection; 14] = [
    MID[0],
    MID[1],
    MID[2],
    MID[3],
    TOP[0],
    TOP[1],
    TOP[2],
    TOP[3],
    BOT[0],
    BOT[1],
    BOT[2],
    BOT[3],
    Protection {
        guarded: tower(rules::LANE_MID, 4),
        until: Until::AnyOf(&ANY_TIER_THREE),
    },
    Protection {
        guarded: StructureId::Ancient,
        until: Until::Down(tower(rules::LANE_MID, 4)),
    },
];
