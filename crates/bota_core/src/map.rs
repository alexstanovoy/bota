//! The maps a match may be played on.
//!
//! Everything that differs between them and a player may know before the
//! match lives here: where the buildings stand, where the lanes run and
//! which camps the jungle holds. The forest and the ground are published
//! with the match itself.

use bota_proto::{HeroId, MapId, ObjectiveKind, ObjectiveSet, RuneSet, Vec2};

use crate::{
    CampDef, CampKind, OutpostDef, Protection, SIMULATED_OBJECTIVES, camp, objective, rules,
};

/// Full Dota geometry with mid-only waves and bounded hero/tower play.
pub const MAP2_ID: MapId = MapId(2);
/// Hero lives lost across a side's seats before that side loses Map2.
pub const MAP2_DEATH_LIMIT: u16 = 2;
/// Map2 gameplay duration in simulation ticks, excluding pregame.
pub const MAP2_GAME_TICKS: u32 = 20 * 60 * rules::TICKS_PER_SECOND;
/// The tick Map2 ends on as a draw when nothing decided it earlier, pregame
/// included.
pub const MAP2_TICK_CAP: u32 = rules::PREGAME_TICKS + MAP2_GAME_TICKS;

/// The kinds of rune the simulation spawns and plays: what a match may ask
/// for. Every other kind is refused until it is simulated.
pub const SIMULATED_RUNES: RuneSet = RuneSet::NONE;

/// One playable map.
#[derive(Clone, Copy, Debug)]
pub struct MapDef {
    /// Which map this is on the wire.
    pub id: MapId,
    /// Fountain centres, Radiant first.
    pub fountains: [Vec2; 2],
    /// Ancient positions, Radiant first. Absent for a side that has none.
    pub ancients: [Option<Vec2>; 2],
    /// Radiant towers as lane, tier and position.
    pub radiant_towers: &'static [(u8, u8, Vec2)],
    /// Dire towers as lane, tier and position.
    pub dire_towers: &'static [(u8, u8, Vec2)],
    /// Barracks as lane, whether ranged, and position; Radiant first. Empty
    /// for a map that runs none.
    pub barracks: [&'static [(u8, bool, Vec2)]; 2],
    /// Filler statue positions; Radiant first. Empty for a map that has none.
    pub fillers: [&'static [Vec2]; 2],
    /// Which structures wait on which, and how.
    pub protection: &'static [Protection],
    /// Where waves appear, by team then lane.
    pub creep_spawns: [[Vec2; 3]; 2],
    /// Geometric lanes, from lane zero up, including lanes without waves.
    pub lanes: u8,
    /// Lanes that spawn creep waves, in spawn order; each index is below `lanes`.
    pub wave_lanes: &'static [u8],
    /// Corners a lane bends through between the two tier-one towers, by lane
    /// index. Empty for a lane that runs straight.
    pub lane_corners: &'static [&'static [Vec2]],
    /// Whether the lane centerline is drawn through the lane's towers. A map
    /// whose corners trace the real road keeps its towers beside it.
    pub lane_through_towers: bool,
    /// The jungle camps.
    pub camps: &'static [CampDef],
    /// Hero deaths on one side that lose it the match. Nought for a map lost
    /// only with a building.
    pub death_limit: u16,
    /// Whether losing any tower loses the match.
    pub tower_ends_it: bool,
    /// The Outposts.
    pub outposts: &'static [OutpostDef],
    /// Where the Watchers stand.
    pub watchers: &'static [Vec2],
    /// Where the Shrines of Wisdom stand.
    pub shrines: &'static [Vec2],
    /// The objectives a match on this map stands up unless its host says
    /// otherwise.
    pub default_objectives: ObjectiveSet,
    /// The runes a match on this map spawns unless its host says otherwise.
    pub default_runes: RuneSet,
    /// The hero a seat that picks none is given, when the match allows it
    /// and no seat it may not share it with holds it. Absent, such a seat is
    /// given a random one of those it may pick, as Dota gives one.
    pub default_hero: Option<HeroId>,
}

/// The Dota lanes bend once each on the way round the map; mid runs straight.
const DOTA_CORNERS: &[&[Vec2]] = &[&[], &[rules::TOP_CORNER], &[rules::BOT_CORNER]];

/// The demo lane bends through its own path corners, straight from the map.
const DEMO_CORNERS: &[&[Vec2]] = &[&rules::DEMO_LANE_CORNERS];

/// The demo map's two camps, in the wooded pockets either side of the
/// lane, both pullable.
const DEMO_CAMPS: [CampDef; 2] = [
    camp(
        Vec2::from_ints(8992, 7968),
        CampKind::Small,
        true,
        6,
        0,
        camp::spawn_box(8692, 7668, 9292, 8268),
    ),
    camp(
        Vec2::from_ints(8864, 9952),
        CampKind::Small,
        true,
        6,
        0,
        camp::spawn_box(8564, 9652, 9164, 10252),
    ),
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
    fillers: [&rules::RADIANT_FILLERS, &rules::DIRE_FILLERS],
    protection: &crate::DOTA_PROTECTION,
    creep_spawns: [rules::RADIANT_CREEP_SPAWNS, rules::DIRE_CREEP_SPAWNS],
    lanes: 3,
    wave_lanes: &[rules::LANE_MID, rules::LANE_TOP, rules::LANE_BOT],
    lane_corners: DOTA_CORNERS,
    lane_through_towers: true,
    camps: &crate::CAMPS,
    death_limit: 0,
    tower_ends_it: false,
    outposts: &objective::DOTA_OUTPOSTS,
    watchers: &objective::DOTA_WATCHERS,
    shrines: &objective::DOTA_SHRINES,
    default_objectives: SIMULATED_OBJECTIVES,
    default_runes: SIMULATED_RUNES,
    default_hero: None,
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
    fillers: [&[], &[]],
    protection: &[],
    creep_spawns: [
        [rules::DEMO_RADIANT_CREEP_SPAWN; 3],
        [rules::DEMO_DIRE_CREEP_SPAWN; 3],
    ],
    lanes: 1,
    wave_lanes: &[rules::LANE_MID],
    lane_corners: DEMO_CORNERS,
    lane_through_towers: false,
    camps: &DEMO_CAMPS,
    death_limit: 0,
    tower_ends_it: false,
    outposts: &[],
    watchers: &[],
    shrines: &[],
    default_objectives: ObjectiveSet::NONE,
    default_runes: RuneSet::NONE,
    default_hero: None,
};

/// Full Dota geometry with mid-only waves and a hero/tower finish bounded by
/// [`MAP2_TICK_CAP`]; the side tier ones stand invulnerable as in Dota's
/// "1v1 Solo Mid", so only a mid tower can end it.
///
/// It holds every objective of the Dota map, but stands none and spawns no
/// rune unless its host asks. Dota's Solo Mid plays the whole map: its rules
/// (`dota_play_1v1_rules_html`) remove none of them and only the first rune;
/// this map keeps the game it played before they were simulated. Its 1v1
/// is a Shadow Fiend mirror, so a seat that picks none plays Shadow Fiend.
const SKIRMISH: MapDef = MapDef {
    id: MAP2_ID,
    protection: &crate::SOLO_MID_PROTECTION,
    wave_lanes: &[rules::LANE_MID],
    death_limit: MAP2_DEATH_LIMIT,
    tower_ends_it: true,
    default_objectives: ObjectiveSet::NONE,
    default_runes: RuneSet::NONE,
    default_hero: Some(HeroId::SHADOW_FIEND),
    ..DOTA
};

/// The map a match is played on. An unknown id falls back on the Dota map.
#[inline]
pub fn map_of(id: MapId) -> &'static MapDef {
    MAPS.iter().find(|m| m.id == id).unwrap_or(&MAPS[0])
}

impl MapDef {
    /// Every geometric lane, including lanes without creep waves.
    pub fn lanes(&self) -> impl Iterator<Item = u8> {
        0..self.lanes
    }

    /// The objectives the map has the data to stand up: those it has a
    /// place for.
    pub const fn objectives_held(&self) -> ObjectiveSet {
        let mut held = ObjectiveSet::NONE;
        if !self.outposts.is_empty() {
            held = held.with(ObjectiveKind::Outposts);
        }
        if !self.watchers.is_empty() {
            held = held.with(ObjectiveKind::Watchers);
        }
        if !self.shrines.is_empty() {
            held = held.with(ObjectiveKind::Shrines);
        }
        held
    }

    /// The kinds of rune the map has spots for: none yet.
    pub const fn runes_held(&self) -> RuneSet {
        RuneSet::NONE
    }

    /// The objectives a match on this map may stand up: those it holds the
    /// data for and the simulation plays.
    pub const fn playable_objectives(&self) -> ObjectiveSet {
        self.objectives_held().and(SIMULATED_OBJECTIVES)
    }

    /// The runes a match on this map may spawn: those it has spots for and
    /// the simulation plays.
    pub const fn playable_runes(&self) -> RuneSet {
        self.runes_held().and(SIMULATED_RUNES)
    }

    /// This map's place in per-map tables: its id.
    pub fn index(&self) -> usize {
        self.id.0 as usize
    }
}
