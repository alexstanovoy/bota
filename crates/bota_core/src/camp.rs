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

impl CampKind {
    /// How many sizes there are.
    pub const COUNT: usize = 4;
}

const _: () = assert!(CampKind::Ancient as usize + 1 == CampKind::COUNT);

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
    /// Whether it stands in a stream: filled with amphibians that evolve
    /// with the clock rather than from the rosters of its size
    /// ([`flooded_creeps`](crate::flooded_creeps)).
    pub flooded: bool,
    /// How many rosters of its size it draws from, the first ones of
    /// [`ROSTERS`](crate::ROSTERS): the map's `maxspawntype`. Zero for a
    /// flooded camp.
    pub rosters: u8,
    /// How many whole sizes a flooded camp grows by: the map's
    /// `maxupgradecount`. Zero for any other.
    pub evolutions: u8,
    /// The camp's spawn box: while a unit stands in it on the minute mark,
    /// the camp puts nothing out.
    pub spawn_box: SpawnBox,
}

/// A camp's spawn box: the footprint of the map's `trigger_multiple` named
/// like the camp, an axis-aligned rectangle (the volume reaches up and down
/// beyond any unit's height).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnBox {
    /// The corner of least x and y.
    pub min: Vec2,
    /// The corner of greatest x and y.
    pub max: Vec2,
}

impl SpawnBox {
    /// Whether a point lies in the box, edges included.
    pub fn contains(&self, point: Vec2) -> bool {
        self.min.x <= point.x
            && point.x <= self.max.x
            && self.min.y <= point.y
            && point.y <= self.max.y
    }
}

/// A box from its corners' coordinates: least x, least y, greatest x,
/// greatest y.
pub(crate) const fn spawn_box(x0: i32, y0: i32, x1: i32, y1: i32) -> SpawnBox {
    SpawnBox {
        min: Vec2::from_ints(x0, y0),
        max: Vec2::from_ints(x1, y1),
    }
}

/// A camp from its fields, in declaration order.
pub(crate) const fn camp(
    pos: Vec2,
    kind: CampKind,
    pullable: bool,
    rosters: u8,
    evolutions: u8,
    spawn_box: SpawnBox,
) -> CampDef {
    CampDef {
        spawn_box,
        pos,
        kind,
        pullable,
        flooded: rosters == 0,
        rosters,
        evolutions,
    }
}

/// Every camp on the map, in the order the map lists them.
///
/// Positions, size classes, the pullable flag, the rosters drawn from and
/// the evolutions of a flooded camp are the map's own
/// `npc_dota_neutral_spawner` entities (client 6957): `origin`,
/// `NeutralType`, `AggroType`, `maxspawntype` (`minspawntype` is 1 for every
/// dry camp; spawn type 7 is the amphibians') and `maxupgradecount`. The
/// spawn boxes are the vertices of the physics hull in the model of each
/// camp's `trigger_multiple` (`maps/dota/entities/neutralcamp_*.vmdl`, a box
/// of the entity's `origin` plus the hull, rounded to whole units), read
/// from the game's `dota.vpk`; the map's frame is shifted by 9216 on both axes.
pub const CAMPS: [CampDef; 28] = [
    // neutralcamp_evil_6
    camp(
        Vec2::from_ints(8364, 14156),
        CampKind::Medium,
        false,
        0,
        2,
        spawn_box(8000, 13736, 8900, 14520),
    ),
    // neutralcamp_evil_9
    camp(
        Vec2::from_ints(12608, 7808),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(12224, 7680, 12992, 8320),
    ),
    // neutralcamp_good_1
    camp(
        Vec2::from_ints(13194, 4189),
        CampKind::Small,
        true,
        6,
        0,
        spawn_box(12863, 3778, 13659, 4568),
    ),
    // neutralcamp_evil_7
    camp(
        Vec2::from_ints(17144, 9096),
        CampKind::Large,
        false,
        6,
        0,
        spawn_box(16833, 8807, 17473, 9511),
    ),
    // neutralcamp_evil_1
    camp(
        Vec2::from_ints(4392, 13131),
        CampKind::Large,
        true,
        6,
        0,
        spawn_box(4004, 12800, 4720, 13614),
    ),
    // neutralcamp_good_2
    camp(
        Vec2::from_ints(13865, 5517),
        CampKind::Large,
        true,
        6,
        0,
        spawn_box(13595, 4903, 14338, 5721),
    ),
    // neutralcamp_good_5
    camp(
        Vec2::from_ints(7762, 5860),
        CampKind::Large,
        false,
        6,
        0,
        spawn_box(7180, 5335, 8228, 6182),
    ),
    // neutralcamp_good_4
    camp(
        Vec2::from_ints(9402, 4019),
        CampKind::Medium,
        false,
        0,
        2,
        spawn_box(8965, 3684, 9852, 4440),
    ),
    // neutralcamp_good_8
    camp(
        Vec2::from_ints(4201, 9120),
        CampKind::Ancient,
        false,
        5,
        0,
        spawn_box(3867, 8624, 4971, 9601),
    ),
    // neutralcamp_evil_8
    camp(
        Vec2::from_ints(13568, 9264),
        CampKind::Ancient,
        false,
        5,
        0,
        spawn_box(12744, 8814, 13886, 9744),
    ),
    // neutralcamp_good_9
    camp(
        Vec2::from_ints(7233, 4401),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(6912, 4128, 7597, 4823),
    ),
    // neutralcamp_evil_4
    camp(
        Vec2::from_ints(10440, 13392),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(9999, 12988, 10737, 13636),
    ),
    // neutralcamp_good_7
    camp(
        Vec2::from_ints(5203, 10208),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(4848, 9751, 5520, 10471),
    ),
    // neutralcamp_good_11
    camp(
        Vec2::from_ints(8496, 1520),
        CampKind::Medium,
        false,
        0,
        1,
        spawn_box(8148, 1197, 8747, 1866),
    ),
    // neutralcamp_evil_14
    camp(
        Vec2::from_ints(9552, 16912),
        CampKind::Medium,
        false,
        0,
        1,
        spawn_box(9176, 16574, 9870, 17278),
    ),
    // neutralcamp_evil_5
    camp(
        Vec2::from_ints(10280, 11796),
        CampKind::Large,
        false,
        6,
        0,
        spawn_box(9853, 11358, 10755, 12147),
    ),
    // neutralcamp_evil_13
    camp(
        Vec2::from_ints(17646, 10479),
        CampKind::Small,
        false,
        5,
        0,
        spawn_box(17408, 10112, 17920, 10752),
    ),
    // neutralcamp_good_13
    camp(
        Vec2::from_ints(903, 8663),
        CampKind::Large,
        false,
        6,
        0,
        spawn_box(571, 8401, 1339, 9169),
    ),
    // neutralcamp_evil_11
    camp(
        Vec2::from_ints(6336, 16592),
        CampKind::Small,
        false,
        0,
        1,
        spawn_box(5897, 16276, 6607, 16980),
    ),
    // neutralcamp_evil_15
    camp(
        Vec2::from_ints(6620, 13066),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(6293, 12759, 7003, 13387),
    ),
    // neutralcamp_good_14
    camp(
        Vec2::from_ints(11984, 880),
        CampKind::Small,
        false,
        0,
        1,
        spawn_box(11560, 502, 12215, 1189),
    ),
    // neutralcamp_good_15
    camp(
        Vec2::from_ints(11138, 5241),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(10674, 4768, 11422, 5598),
    ),
    // neutralcamp_evil_2
    camp(
        Vec2::from_ints(5305, 14045),
        CampKind::Small,
        true,
        6,
        0,
        spawn_box(4956, 13766, 5596, 14557),
    ),
    // neutralcamp_good_16
    camp(
        Vec2::from_ints(1193, 7378),
        CampKind::Small,
        false,
        5,
        0,
        spawn_box(859, 7043, 1608, 7824),
    ),
    // neutralcamp_evil_12
    camp(
        Vec2::from_ints(11232, 17112),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(10845, 16740, 11747, 17484),
    ),
    // neutralcamp_good_12
    camp(
        Vec2::from_ints(6801, 814),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(6360, 448, 7112, 1184),
    ),
    // neutralcamp_good_20
    camp(
        Vec2::from_ints(13632, 784),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(13288, 582, 13943, 1269),
    ),
    // neutralcamp_evil_20
    camp(
        Vec2::from_ints(5008, 17552),
        CampKind::Medium,
        false,
        5,
        0,
        spawn_box(4621, 17120, 5331, 17824),
    ),
];
