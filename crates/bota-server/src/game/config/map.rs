//! The maps a match may be played on.
//!
//! Everything that differs between them lives here: where the buildings
//! stand, where the lanes run, which camps the jungle holds, and whether the
//! ground is the real terrain or open field.

use bota_proto::{MapId, Vec2};

use crate::game::{CampDef, CampKind, Protection, rules};

/// One playable map.
#[derive(Clone, Copy, Debug)]
pub struct MapDef {
    /// Which map this is on the wire.
    pub id: MapId,
    /// Fountain centres, Radiant first.
    pub fountains: [Vec2; 2],
    /// Ancient positions, Radiant first. Absent for a side that has none;
    /// a match with no Ancients runs until its clock says otherwise.
    pub ancients: [Option<Vec2>; 2],
    /// Radiant towers as lane, tier and position.
    pub radiant_towers: &'static [(u8, u8, Vec2)],
    /// Dire towers as lane, tier and position.
    pub dire_towers: &'static [(u8, u8, Vec2)],
    /// Barracks as lane, whether ranged, and position; Radiant first. Empty
    /// for a map that runs none.
    pub barracks: [&'static [(u8, bool, Vec2)]; 2],
    /// Which structures wait on which, and how.
    pub protection: &'static [Protection],
    /// Where waves appear, by team then lane.
    pub creep_spawns: [[Vec2; 3]; 2],
    /// How many lanes the map runs, from lane zero up.
    pub lanes: u8,
    /// Corners a lane bends through between the two tier-one towers, by lane
    /// index. Empty for a lane that runs straight.
    pub lane_corners: &'static [&'static [Vec2]],
    /// Whether the lane centerline is drawn through the lane's towers. A map
    /// whose corners trace the real road keeps its towers beside it.
    pub lane_through_towers: bool,
    /// The jungle camps.
    pub camps: &'static [CampDef],
    /// The forest, tree by tree. Empty for a map with none.
    pub trees: &'static [(i16, i16)],
    /// Trees this close to a lane centerline are dropped, in world units.
    /// Zero keeps every tree: a map whose corners follow the real roads has
    /// nothing standing on them.
    pub lane_clear: i32,
    /// The map's own vision blocker walls. Empty for a map with none.
    pub fow_blockers: &'static [&'static [(i16, i16)]],
    /// The baked ground, run-length encoded: walkability, elevation tiers
    /// and water, cell by cell.
    pub terrain_rle: &'static [(u16, u8)],
    /// Hero deaths on one side that lose it the match. Nought for a map lost
    /// only with a building.
    pub death_limit: u16,
    /// Whether losing any tower loses the match.
    pub tower_ends_it: bool,
}

/// The Dota lanes bend once each on the way round the map; mid runs straight.
const DOTA_CORNERS: &[&[Vec2]] = &[&[], &[rules::TOP_CORNER], &[rules::BOT_CORNER]];

/// The demo lane bends through its own path corners, straight from the map.
const DEMO_CORNERS: &[&[Vec2]] = &[&rules::DEMO_LANE_CORNERS];

const fn camp(pos: Vec2, kind: CampKind, pullable: bool, flooded: bool) -> CampDef {
    CampDef {
        pos,
        kind,
        pullable,
        flooded,
    }
}

/// The demo map's two camps, in the wooded pockets either side of the
/// lane, both pullable.
///
/// The real demo map runs no jungle; these stand in so everything the
/// jungle does can be read off the small map too.
const DEMO_CAMPS: [CampDef; 2] = [
    camp(Vec2::from_ints(8992, 7968), CampKind::Small, true, false),
    camp(Vec2::from_ints(8864, 9952), CampKind::Small, true, false),
];

/// Every map, indexed by [`MapId`].
pub const MAPS: [MapDef; 3] = [DOTA, DEMO, SKIRMISH];

/// The Dota map, played to its Ancient.
const DOTA: MapDef = MapDef {
    id: MapId(0),
    fountains: [rules::RADIANT_FOUNTAIN_POS, rules::DIRE_FOUNTAIN_POS],
    ancients: [
        Some(rules::RADIANT_ANCIENT_POS),
        Some(rules::DIRE_ANCIENT_POS),
    ],
    radiant_towers: &rules::RADIANT_TOWERS,
    dire_towers: &rules::DIRE_TOWERS,
    barracks: [&rules::RADIANT_BARRACKS, &rules::DIRE_BARRACKS],
    protection: &crate::game::DOTA_PROTECTION,
    creep_spawns: [rules::RADIANT_CREEP_SPAWNS, rules::DIRE_CREEP_SPAWNS],
    lanes: 3,
    lane_corners: DOTA_CORNERS,
    lane_through_towers: true,
    camps: &crate::game::CAMPS,
    trees: crate::game::DOTA_TREES,
    lane_clear: rules::TREE_LANE_CLEAR,
    fow_blockers: crate::game::FOW_BLOCKERS,
    terrain_rle: crate::game::TERRAIN_RLE,
    death_limit: 0,
    tower_ends_it: false,
};

/// The hero demo map: one short lane with a single tower a side, two
/// fountains, no Ancients, and the real forest and ground. Everything
/// comes from the game's own `hero_demo_main`, shifted like the big map.
const DEMO: MapDef = MapDef {
    id: MapId(1),
    fountains: [
        rules::DEMO_RADIANT_FOUNTAIN_POS,
        rules::DEMO_DIRE_FOUNTAIN_POS,
    ],
    ancients: [None, None],
    radiant_towers: &rules::DEMO_RADIANT_TOWERS,
    dire_towers: &rules::DEMO_DIRE_TOWERS,
    barracks: [&[], &[]],
    protection: &[],
    creep_spawns: [
        [rules::DEMO_RADIANT_CREEP_SPAWN; 3],
        [rules::DEMO_DIRE_CREEP_SPAWN; 3],
    ],
    lanes: 1,
    lane_corners: DEMO_CORNERS,
    lane_through_towers: false,
    camps: &DEMO_CAMPS,
    trees: crate::game::DEMO_TREES,
    lane_clear: 0,
    fow_blockers: crate::game::DEMO_FOW_BLOCKERS,
    terrain_rle: crate::game::DEMO_TERRAIN_RLE,
    death_limit: 0,
    tower_ends_it: false,
};

/// The Dota map played to a short finish: the first side to lose a tower or
/// to lose [`rules::SKIRMISH_DEATH_LIMIT`] heroes loses the match.
///
/// The same ground and the same buildings as the Dota map; only what ends it
/// differs, which is what makes a match on it worth reading against one on
/// map nought.
const SKIRMISH: MapDef = MapDef {
    id: MapId(2),
    death_limit: rules::SKIRMISH_DEATH_LIMIT,
    tower_ends_it: true,
    ..DOTA
};

/// The map a match is played on. An unknown id falls back on the Dota map.
pub fn map_of(id: MapId) -> &'static MapDef {
    MAPS.iter().find(|m| m.id == id).unwrap_or(&MAPS[0])
}

impl MapDef {
    /// Every lane the map runs.
    pub fn lanes(&self) -> impl Iterator<Item = u8> {
        0..self.lanes
    }

    /// Where this map's index sits in the per-team tables.
    pub fn index(&self) -> usize {
        self.id.0 as usize
    }
}
