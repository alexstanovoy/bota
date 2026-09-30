//! The jungle camps of the map, as the map itself declares them.

use bota_proto::Vec2;

/// The size class of a neutral camp: which rosters it draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CampKind {
    /// Three creeps, five in the kobold camp. Six rosters.
    Small,
    /// Two to four creeps. Five rosters.
    Medium,
    /// Two or three creeps. Six rosters.
    Large,
    /// Three creeps of the ancient unit type.
    Ancient,
}

/// One jungle camp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CampDef {
    /// Where the camp spawns, at the center of its box.
    pub pos: Vec2,
    /// The size class.
    pub kind: CampKind,
    /// Whether lane creeps will engage this camp's neutrals. True on the
    /// four camps the map marks with an aggro type of one.
    pub pullable: bool,
    /// Whether the map marks this camp as filled from the amphibian roster.
    /// Read by nothing.
    pub flooded: bool,
}

const fn camp(pos: Vec2, kind: CampKind, pullable: bool, flooded: bool) -> CampDef {
    CampDef {
        pos,
        kind,
        pullable,
        flooded,
    }
}

/// Every camp on the map, in the order the map lists them.
///
/// Positions, size classes and the pullable flag are the map's own
/// `npc_dota_neutral_spawner` entities: `origin`, `NeutralType` and
/// `AggroType`.
pub const CAMPS: [CampDef; 28] = [
    // neutralcamp_evil_6
    camp(Vec2::from_ints(8364, 14156), CampKind::Medium, false, true),
    // neutralcamp_evil_9
    camp(Vec2::from_ints(12608, 7808), CampKind::Medium, false, false),
    // neutralcamp_good_1
    camp(Vec2::from_ints(13194, 4189), CampKind::Small, true, false),
    // neutralcamp_evil_7
    camp(Vec2::from_ints(17144, 9096), CampKind::Large, false, false),
    // neutralcamp_evil_1
    camp(Vec2::from_ints(4392, 13131), CampKind::Large, true, false),
    // neutralcamp_good_2
    camp(Vec2::from_ints(13865, 5517), CampKind::Large, true, false),
    // neutralcamp_good_5
    camp(Vec2::from_ints(7762, 5860), CampKind::Large, false, false),
    // neutralcamp_good_4
    camp(Vec2::from_ints(9402, 4019), CampKind::Medium, false, true),
    // neutralcamp_good_8
    camp(Vec2::from_ints(4201, 9120), CampKind::Ancient, false, false),
    // neutralcamp_evil_8
    camp(
        Vec2::from_ints(13568, 9264),
        CampKind::Ancient,
        false,
        false,
    ),
    // neutralcamp_good_9
    camp(Vec2::from_ints(7233, 4401), CampKind::Medium, false, false),
    // neutralcamp_evil_4
    camp(
        Vec2::from_ints(10440, 13392),
        CampKind::Medium,
        false,
        false,
    ),
    // neutralcamp_good_7
    camp(Vec2::from_ints(5203, 10208), CampKind::Medium, false, false),
    // neutralcamp_good_11
    camp(Vec2::from_ints(8496, 1520), CampKind::Medium, false, true),
    // neutralcamp_evil_14
    camp(Vec2::from_ints(9552, 16912), CampKind::Medium, false, true),
    // neutralcamp_evil_5
    camp(Vec2::from_ints(10280, 11796), CampKind::Large, false, false),
    // neutralcamp_evil_13
    camp(Vec2::from_ints(17646, 10479), CampKind::Small, false, false),
    // neutralcamp_good_13
    camp(Vec2::from_ints(903, 8663), CampKind::Large, false, false),
    // neutralcamp_evil_11
    camp(Vec2::from_ints(6336, 16592), CampKind::Small, false, true),
    // neutralcamp_evil_15
    camp(Vec2::from_ints(6620, 13066), CampKind::Medium, false, false),
    // neutralcamp_good_14
    camp(Vec2::from_ints(11984, 880), CampKind::Small, false, true),
    // neutralcamp_good_15
    camp(Vec2::from_ints(11138, 5241), CampKind::Medium, false, false),
    // neutralcamp_evil_2
    camp(Vec2::from_ints(5305, 14045), CampKind::Small, true, false),
    // neutralcamp_good_16
    camp(Vec2::from_ints(1193, 7378), CampKind::Small, false, false),
    // neutralcamp_evil_12
    camp(
        Vec2::from_ints(11232, 17112),
        CampKind::Medium,
        false,
        false,
    ),
    // neutralcamp_good_12
    camp(Vec2::from_ints(6801, 814), CampKind::Medium, false, false),
    // neutralcamp_good_20
    camp(Vec2::from_ints(13632, 784), CampKind::Medium, false, false),
    // neutralcamp_evil_20
    camp(Vec2::from_ints(5008, 17552), CampKind::Medium, false, false),
];
