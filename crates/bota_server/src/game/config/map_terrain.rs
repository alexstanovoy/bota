//! What of each map is published with the match rather than known before
//! it: the forest and the ground.

use bota_core::{MAPS, MapDef, rules};

use crate::game::{
    DEMO_FOW_BLOCKERS, DEMO_TERRAIN_RLE, DEMO_TREES, DOTA_TREES, FOW_BLOCKERS, TERRAIN_RLE,
};

/// The forest and the ground of one map.
#[derive(Clone, Copy, Debug)]
pub struct MapTerrain {
    /// The forest, tree by tree. Empty for a map with none.
    pub trees: &'static [(i16, i16)],
    /// Trees this close to a lane centerline are dropped, in world units.
    /// Zero keeps every tree.
    pub lane_clear: i32,
    /// The map's own vision blocker walls. Empty for a map with none.
    pub fow_blockers: &'static [&'static [(i16, i16)]],
    /// The baked ground, run-length encoded: walkability, elevation tiers
    /// and water, cell by cell.
    pub terrain_rle: &'static [(u16, u8)],
}

/// The Dota map's forest and ground, shared by every map played on it.
pub const DOTA_TERRAIN: MapTerrain = MapTerrain {
    trees: DOTA_TREES,
    lane_clear: rules::TREE_LANE_CLEAR,
    fow_blockers: FOW_BLOCKERS,
    terrain_rle: TERRAIN_RLE,
};

/// Every map's forest and ground, indexed like [`MAPS`].
const TERRAINS: [MapTerrain; MAPS.len()] = [DOTA_TERRAIN, DEMO_TERRAIN, DOTA_TERRAIN];

/// The hero demo map's forest and ground.
pub const DEMO_TERRAIN: MapTerrain = MapTerrain {
    trees: DEMO_TREES,
    lane_clear: 0,
    fow_blockers: DEMO_FOW_BLOCKERS,
    terrain_rle: DEMO_TERRAIN_RLE,
};

/// The forest and ground of a map.
pub fn terrain_of(map: &MapDef) -> &'static MapTerrain {
    &TERRAINS[map.index()]
}
