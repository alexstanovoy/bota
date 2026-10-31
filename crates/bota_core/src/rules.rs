//! Balance constants.
//!
//! Time is in ticks unless a doc says milliseconds or seconds, distances in
//! whole world units. Time constants assume [`TICKS_PER_SECOND`]; the
//! wall-clock pace of a match is a server option and does not change them.

use bota_proto::{Attributes, Fixed, Vec2};

use crate::GoldRange;

// Hero numbers live with their heroes and are re-exported here, so a
// constant is `rules::NAME` wherever it is kept.
pub use crate::heroes::pudge::numbers::*;
pub use crate::heroes::shadow_fiend::numbers::*;
// The protocol's bounds are `bota_proto::limits`; the rules read them as `rules::NAME` too.
pub use bota_proto::limits::{
    BACKPACK_SLOTS, INVENTORY_SLOTS, MAX_ANNOUNCED_NAME_BYTES, MAX_CHAT_BYTES, MAX_CHAT_GRAPHEMES,
    MAX_EFFECTS_PER_UNIT, MAX_ORDER_DELAY_TICKS, MAX_ORDERS_PER_TICK, MAX_QUEUED_ORDERS,
    MAX_SHOWN_NAME_BYTES, STASH_SLOTS, TOWN_PORTAL_SLOT,
};

/// Simulation ticks per second of game time.
pub const TICKS_PER_SECOND: u32 = 30;

/// What a rate or an amplification is worth at nominal, in basis points.
pub const NOMINAL_BP: i32 = 10_000;

/// The lowest magic resistance anything can be brought to, in basis points:
/// magic damage at most doubled.
pub const MIN_MAGIC_RESIST_BP: i32 = -NOMINAL_BP;

/// The map spans `0..MAP_SIZE` on both axes.
pub const MAP_SIZE: i32 = 18432;

/// Cells per axis of the passability grid; matches the terrain layout.
pub const GRID_CELLS: usize = 288;

/// World units covered by one passability cell.
pub const GRID_CELL_SIZE: i32 = MAP_SIZE / GRID_CELLS as i32;

// The ground as a body meets it.

/// World units between neighbouring nodes of the walking lattice, on which
/// routes are planned and the room a body has is kept: half Dota's gridnav
/// cell (dota.fgd `@gridnav(64, ...)`), its nodes at every cell's centre
/// and at the midpoints and corners between them. The long pather's way
/// through a forest runs through gridnav cell centres: a lane one cell wide
/// between two trees (which stand on cell corners and close the 128x128
/// square about them, Liquipedia *Trees*) has its nodes on its middle line.
pub const WALK_CELL_SIZE: i32 = GRID_CELL_SIZE / 2;
/// Nodes per axis of the walking lattice, the map's far edge included.
pub const WALK_CELLS: usize = (MAP_SIZE / WALK_CELL_SIZE) as usize + 1;
/// The most room a node records, in world units. Everything further from
/// every obstacle than this reads the same.
pub const CLEARANCE_CAP: i32 = 96;
/// World units covered by one bucket of the obstacle and body indexes.
pub const BUCKET_SIZE: i32 = 256;
/// Buckets per axis of the obstacle and body indexes.
pub const BUCKETS: usize = (MAP_SIZE / BUCKET_SIZE) as usize;
/// How far a route corner is drawn in towards the obstacle it rounds, at
/// most, in world units.
pub const TIGHTEN_MAX: i32 = 48;
/// Nodes a route search expands before it settles for the node nearest
/// its goal.
pub const PATH_EXPANSIONS: u32 = 20_000;
/// How far ahead along its route a walker plans its next stretch, in world
/// units.
pub const LOCAL_REACH: i32 = 440;
/// Ticks one straight stretch of a local plan lasts.
pub const LOCAL_PRIM_TICKS: u32 = 4;
/// Ticks a local plan reaches into the future, at most.
pub const LOCAL_HORIZON_TICKS: u32 = 40;
/// States a local search expands before it settles for the best it got to.
pub const LOCAL_EXPANSIONS: u32 = 120;
/// World units of the lattice two local states are told apart by.
pub const LOCAL_KEY_CELL: i32 = 32;
/// Dota's `dota_unit_blocked_angle_threshold` (0.1), read as the cosine,
/// in thousandths, of the angle between a walker's step and the way to the
/// body that cuts it within which that body blocks the walk: the walker
/// waits on it ([`UNIT_BLOCKED_WAIT_TICKS`]) before laying its way round
/// it. 0.1 is some 84 degrees, the whole front of the walker.
/// Approximation: the console names the value, not its unit; read as
/// radians it would leave a 6-degree sliver, and a creep met anywhere off
/// dead centre would go round a standing blocker at once.
pub const UNIT_BLOCKED_COS_PERMILLE: i64 = 100;
/// Dota's `dota_unit_blocked_look_ahead_angle_threshold` (0.7), read the
/// same way: the body a walker waits on is waited on only while it stands
/// within some 46 degrees of the way the walker goes; one that stands
/// beside the way its short pather lays round at once. Approximation as for
/// [`UNIT_BLOCKED_COS_PERMILLE`], and which of the walker's ways the look
/// ahead is taken along (here: to the aim of its plan) is not documented.
pub const UNIT_BLOCKED_LOOK_AHEAD_COS_PERMILLE: i64 = 700;
/// Dota's `dota_unit_blocked_wait_time`, `_mid` and `_long` (0.15, 0.3 and
/// 0.4 s) in ticks, rounded up: how long a blocked walker waits before it
/// lays its way round the body that blocks it, the first time, once that
/// way was cut by it again, and every time after. A free step ends the
/// wait. Approximation: the order the three are used in is read from
/// their names; Dota does not document it.
pub const UNIT_BLOCKED_WAIT_TICKS: [u32; 3] = [
    (150 * TICKS_PER_SECOND).div_ceil(1000),
    (300 * TICKS_PER_SECOND).div_ceil(1000),
    (400 * TICKS_PER_SECOND).div_ceil(1000),
];
/// Ticks after a local plan is laid before it is laid again for coming out
/// empty or running short.
pub const REPLAN_MIN_TICKS: u32 = 4;
/// [`REPLAN_MIN_TICKS`] for a body that has stood stalled for
/// [`STALL_BACKOFF_AFTER`] ticks or more.
pub const REPLAN_STALLED_TICKS: u32 = 16;
/// Ticks stalled from which a body waits [`REPLAN_STALLED_TICKS`] rather
/// than [`REPLAN_MIN_TICKS`].
pub const STALL_BACKOFF_AFTER: u32 = 30;
/// A local plan laid to a spot short of the route's end is laid again once
/// fewer ticks than this are left of it.
pub const REPLAN_LEFT_TICKS: u32 = 12;
/// A local plan is laid again once the route goal it was laid for has
/// moved this far, in world units.
pub const PLAN_DRIFT: i32 = 64;
/// How far beyond one tick's step a body may stand from its plan's next
/// step and still walk the plan, in world units.
pub const PLAN_STRAY: i32 = 8;
/// Nodes a route search laid round standing bodies expands at most.
pub const STALL_PATH_EXPANSIONS: u32 = 600;
/// How far the body index is searched past the reach asked, in world units,
/// for bodies that moved since it was laid.
pub const BODY_INDEX_SLACK: i32 = 32;

// Landmarks: the current Dota 2 map, every position shifted by half the map
// so Dota's origin sits at the center. The two sides are not mirror images;
// each carries its own table.

/// Center of the Radiant fountain area.
pub const RADIANT_FOUNTAIN_POS: Vec2 = Vec2::from_ints(1760, 2278);
/// Center of the Dire fountain area.
pub const DIRE_FOUNTAIN_POS: Vec2 = Vec2::from_ints(16624, 16064);
pub const RADIANT_ANCIENT_POS: Vec2 = Vec2::from_ints(3296, 3864);
pub const DIRE_ANCIENT_POS: Vec2 = Vec2::from_ints(14744, 14216);
/// How far from the fountain center a hero appears, along both axes towards
/// the map center.
pub const HERO_SPAWN_OFFSET: i32 = 280;

// Lanes. Lane 0 is mid across the middle; lane 1 is top, up the west edge
// and along the north edge; lane 2 is bottom along the south and east.

pub const LANE_MID: u8 = 0;
pub const LANE_TOP: u8 = 1;
/// The bottom lane.
pub const LANE_BOT: u8 = 2;
/// Corner of the top lane, where the west edge meets the north edge.
pub const TOP_CORNER: Vec2 = Vec2::from_ints(3050, 15150);
/// Corner of the bottom lane.
pub const BOT_CORNER: Vec2 = Vec2::from_ints(15400, 2900);
/// A lane waypoint counts as passed within this distance.
pub const LANE_WAYPOINT_RADIUS: i32 = 250;

/// Radiant towers: lane, tier and position, straight from the map.
pub const RADIANT_TOWERS: [(u8, u8, Vec2); 11] = [
    (LANE_MID, 1, Vec2::from_ints(7672, 7808)),
    (LANE_MID, 2, Vec2::from_ints(6026, 6290)),
    (LANE_MID, 3, Vec2::from_ints(4576, 5072)),
    (LANE_TOP, 1, Vec2::from_ints(2880, 11072)),
    (LANE_TOP, 2, Vec2::from_ints(2715, 8344)),
    (LANE_TOP, 3, Vec2::from_ints(2624, 5808)),
    (LANE_BOT, 1, Vec2::from_ints(14076, 2837)),
    (LANE_BOT, 2, Vec2::from_ints(8856, 2960)),
    (LANE_BOT, 3, Vec2::from_ints(5264, 3104)),
    (LANE_MID, 4, Vec2::from_ints(3504, 4352)),
    (LANE_MID, 4, Vec2::from_ints(3824, 4024)),
];

/// Dire towers: lane, tier and position, straight from the map.
pub const DIRE_TOWERS: [(u8, u8, Vec2); 11] = [
    (LANE_MID, 1, Vec2::from_ints(9740, 9868)),
    (LANE_MID, 2, Vec2::from_ints(11712, 11328)),
    (LANE_MID, 3, Vec2::from_ints(13488, 12975)),
    (LANE_TOP, 1, Vec2::from_ints(3941, 15252)),
    (LANE_TOP, 2, Vec2::from_ints(9088, 15232)),
    (LANE_TOP, 3, Vec2::from_ints(12768, 14992)),
    (LANE_BOT, 1, Vec2::from_ints(15485, 6976)),
    (LANE_BOT, 2, Vec2::from_ints(15616, 9600)),
    (LANE_BOT, 3, Vec2::from_ints(15552, 12248)),
    (LANE_MID, 4, Vec2::from_ints(14160, 13992)),
    (LANE_MID, 4, Vec2::from_ints(14496, 13648)),
];

/// Where Radiant creep waves appear, indexed by lane: the map's own lane
/// spawner positions.
pub const RADIANT_CREEP_SPAWNS: [Vec2; 3] = [
    Vec2::from_ints(4208, 4728),
    Vec2::from_ints(2608, 5152),
    Vec2::from_ints(5568, 3104),
];

/// Where Dire creep waves appear, indexed by lane.
pub const DIRE_CREEP_SPAWNS: [Vec2; 3] = [
    Vec2::from_ints(13312, 12800),
    Vec2::from_ints(12384, 15008),
    Vec2::from_ints(15488, 12864),
];

/// Radiant barracks: lane, whether it feeds the ranged creeps, and position,
/// straight from the map.
pub const RADIANT_BARRACKS: [(u8, bool, Vec2); 6] = [
    (LANE_MID, false, Vec2::from_ints(4544, 4664)),
    (LANE_MID, true, Vec2::from_ints(4156, 5017)),
    (LANE_TOP, false, Vec2::from_ints(2880, 5458)),
    (LANE_TOP, true, Vec2::from_ints(2372, 5457)),
    (LANE_BOT, false, Vec2::from_ints(4936, 2856)),
    (LANE_BOT, true, Vec2::from_ints(4937, 3363)),
];

/// Dire barracks: lane, whether it feeds the ranged creeps, and position.
pub const DIRE_BARRACKS: [(u8, bool, Vec2); 6] = [
    (LANE_MID, false, Vec2::from_ints(13918, 13040)),
    (LANE_MID, true, Vec2::from_ints(13552, 13399)),
    (LANE_TOP, false, Vec2::from_ints(13114, 14712)),
    (LANE_TOP, true, Vec2::from_ints(13110, 15241)),
    (LANE_BOT, false, Vec2::from_ints(15808, 12608)),
    (LANE_BOT, true, Vec2::from_ints(15280, 12592)),
];

/// Radiant fillers, straight from the map (`npc_dota_goodguys_fillers`).
pub const RADIANT_FILLERS: [Vec2; 7] = [
    Vec2::from_ints(2626, 5009),
    Vec2::from_ints(3378, 4924),
    Vec2::from_ints(3904, 4416),
    Vec2::from_ints(4434, 3948),
    Vec2::from_ints(4464, 3136),
    Vec2::from_ints(3618, 5505),
    Vec2::from_ints(4951, 4054),
];

/// Dire fillers, straight from the map (`npc_dota_badguys_fillers`).
pub const DIRE_FILLERS: [Vec2; 7] = [
    Vec2::from_ints(15542, 13014),
    Vec2::from_ints(14737, 13165),
    Vec2::from_ints(14110, 13586),
    Vec2::from_ints(13697, 14205),
    Vec2::from_ints(13627, 14969),
    Vec2::from_ints(14502, 12590),
    Vec2::from_ints(13000, 13934),
];

// The hero demo map, `hero_demo_main`: the same frame as the big map, every
// landmark the game's own, shifted by half the map.

/// The demo map's Radiant fountain.
pub const DEMO_RADIANT_FOUNTAIN_POS: Vec2 = Vec2::from_ints(6528, 7040);
/// The demo map's Dire fountain.
pub const DEMO_DIRE_FOUNTAIN_POS: Vec2 = Vec2::from_ints(11470, 10484);
/// The demo map's one Radiant tower.
pub const DEMO_RADIANT_TOWERS: [(u8, u8, Vec2); 1] = [(LANE_MID, 1, Vec2::from_ints(7744, 7872))];
/// The demo map's one Dire tower.
pub const DEMO_DIRE_TOWERS: [(u8, u8, Vec2); 1] = [(LANE_MID, 1, Vec2::from_ints(10240, 9536))];
/// Where the demo map's Radiant wave appears.
pub const DEMO_RADIANT_CREEP_SPAWN: Vec2 = Vec2::from_ints(7360, 7712);
/// Where the demo map's Dire wave appears.
pub const DEMO_DIRE_CREEP_SPAWN: Vec2 = Vec2::from_ints(10432, 9984);
/// The corners the demo lane bends through between the two towers, the
/// map's own path corners.
pub const DEMO_LANE_CORNERS: [Vec2; 3] = [
    Vec2::from_ints(8128, 8512),
    Vec2::from_ints(8768, 9024),
    Vec2::from_ints(9312, 9376),
];

// Trees and the jungle.

/// Tree trunk collision radius. Also what a click has to land inside to be a
/// click on that tree, and how near a tree another may not be planted.
pub const TREE_RADIUS: i32 = 48;
/// Squared world units around a map tree within which a cell's centre is
/// shut to sight. Dota's tree vision blocker is a circle of sqrt(2) tiles through
/// the corners of the 2x2-tile square the tree stands on; the cells whose
/// centres it holds are exactly those four.
pub const TREE_SIGHT_RADIUS_SQUARED: i64 = 2 * GRID_CELL_SIZE as i64 * GRID_CELL_SIZE as i64;
/// Tiers above its own ground a tree still blocks a viewer from: Dota's
/// trees stop ground vision on their level and one level up.
pub const TREE_SIGHT_TIERS_ABOVE: u8 = 1;
/// Squared cell distance from a viewer's cell within which it sees a cell
/// whatever stands between: Dota's 5x5 tiles less the corners.
pub const NEAR_SIGHT_CELLS_SQUARED: i64 = 5;
/// Trees of the Dota map this close to a lane centerline are dropped.
pub const TREE_LANE_CLEAR: i32 = 450;
/// Tree-free radius around each fountain.
pub const TREE_BASE_CLEAR: i32 = 500;

/// A neutral wakes when a hostile unit comes this close to it.
pub const NEUTRAL_AGGRO_RANGE: i32 = 240;
/// A neutral's camp wakes when one of it is struck by something within
/// this range.
pub const NEUTRAL_DAMAGE_AGGRO_RANGE: i32 = 1800;
/// How far around a struck neutral the blow wakes the others (Liquipedia,
/// *Neutral Creeps*, Losing Aggro: "damaging them or another neutral creep
/// within a 500 radius"; the flight from an unseen striker reaches as far).
pub const NEUTRAL_AGGRO_SPREAD: i32 = 500;
/// How far from its spawn spot a neutral may stand before its aggro window
/// starts running down.
pub const NEUTRAL_GUARD_DISTANCE: i32 = 400;
/// Ticks a neutral may spend beyond its guard distance before it walks
/// home.
pub const NEUTRAL_AGGRO_WINDOW: u32 = 5 * TICKS_PER_SECOND;
/// [`NEUTRAL_AGGRO_WINDOW`] of a neutral struck back from its walk home.
pub const NEUTRAL_SHORT_WINDOW: u32 = 3 * TICKS_PER_SECOND;
/// Ticks after a leash break in which nothing wakes a neutral.
pub const NEUTRAL_REAGGRO_BLOCK: u32 = 3 * TICKS_PER_SECOND;
/// Being this close to its spawn spot ends a neutral's walk home.
pub const NEUTRAL_RETURN: i32 = 100;
/// Ticks between neutral upgrades; the same cadence as creep waves.
pub const NEUTRAL_UPGRADE_PERIOD_TICKS: u32 = 450 * TICKS_PER_SECOND;
/// Upgrades a neutral may carry at most.
pub const NEUTRAL_UPGRADE_CAP: i32 = 30;
/// Health one upgrade adds to a neutral.
pub const NEUTRAL_UPGRADE_HP: i32 = 30;
/// Half-points of armor one upgrade adds to a neutral.
pub const NEUTRAL_UPGRADE_ARMOR_HALVES: i32 = 1;
/// Attack damage one upgrade adds to a neutral.
pub const NEUTRAL_UPGRADE_DAMAGE: i32 = 3;
/// Attack speed one neutral upgrade interval adds.
pub const NEUTRAL_UPGRADE_ATTACK_SPEED: i32 = 5;
/// Ticks between the levels a neutral spawns its abilities at: Dota's
/// `neutral_upgrade` ability upgrade interval, 900 s.
pub const NEUTRAL_ABILITY_LEVEL_PERIOD_TICKS: u32 = 900 * TICKS_PER_SECOND;
/// Ticks between the evolutions of a flooded camp: one creep every five
/// minutes from the horn.
pub const FLOODED_EVOLUTION_PERIOD_TICKS: u32 = 5 * 60 * TICKS_PER_SECOND;
/// How far a neutral camp's aura reaches, whatever its ability says: the
/// game shrinks every aura of a neutral it does not hand to a player to
/// this (Liquipedia, the creeps' auras: "Neutral Aura: 800").
pub const NEUTRAL_AURA_RADIUS: i32 = 800;
/// How long the count of blows a Steal Weapon has landed on one enemy lasts
/// after the last of them. The game's files give no figure; it is longer
/// than a fight's gap between two blows and shorter than the ability's wait.
pub const STRIKE_COUNTER_TICKS: u32 = 5 * TICKS_PER_SECOND;
/// Gold bounty one upgrade adds to a neutral.
pub const NEUTRAL_UPGRADE_GOLD: i32 = 1;
/// Experience one upgrade adds to a neutral.
pub const NEUTRAL_UPGRADE_XP: i32 = 5;
/// A wave spawns facing the first waypoint of its lane further than this
/// from its spawn.
pub const WAVE_FACING_LOOKAHEAD: i32 = 200;
/// World units between neighbours in a camp when it spawns.
pub const CAMP_SPAWN_SPACING: i32 = 64;
/// Ticks from the horn to the first neutral spawn: one minute.
pub const FIRST_NEUTRAL_AFTER_HORN: u32 = 60 * TICKS_PER_SECOND;
/// Ticks between neutral spawn checks.
pub const NEUTRAL_SPAWN_PERIOD_TICKS: u32 = 60 * TICKS_PER_SECOND;
/// Neutral creep collision size, the hero hull's.
pub const NEUTRAL_COLLISION: i32 = 27;
/// Neutral creep bound radius, the hero hull's.
pub const NEUTRAL_BOUND: i32 = 24;
/// Neutral creep fog light radius, by day and by night alike unless a kind
/// says otherwise.
pub const NEUTRAL_VISION: i32 = 800;

// Creep waves.

/// Ticks of Solo Mid's pregame: thirty seconds from the heroes coming in to
/// the horn (Dota's 1v1 Solo Mid; its full game has [`DOTA_PREGAME_TICKS`]).
pub const SOLO_MID_PREGAME_TICKS: u32 = 30 * TICKS_PER_SECOND;

/// A match's clock: the tick its game clock reads 0:00 at, the horn, which
/// its map's pregame puts off ([`crate::MapDef::pregame_ticks`]), and every
/// rule timed from the horn. The game clock counts up from minus the
/// pregame, and the first wave walks out at zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Schedule {
    /// The tick of the horn: the pregame's ticks.
    pub horn: u32,
}

impl Schedule {
    /// The schedule of a match whose pregame lasts `pregame_ticks`.
    pub const fn of(pregame_ticks: u32) -> Self {
        Self {
            horn: pregame_ticks,
        }
    }

    /// The tick a game minute begins at, counted from the horn.
    pub const fn at_minute(self, minutes: u32) -> u32 {
        self.horn + minutes * 60 * TICKS_PER_SECOND
    }

    /// The tick of the first creep wave: the horn.
    pub const fn first_wave(self) -> u32 {
        self.horn
    }

    /// The tick of the first neutral spawn.
    pub const fn first_neutral(self) -> u32 {
        self.horn + FIRST_NEUTRAL_AFTER_HORN
    }

    /// The tick from which an attack order may draw any lane creep.
    pub const fn free_aggro(self) -> u32 {
        self.first_wave() + FREE_AGGRO_AFTER_FIRST_WAVE
    }

    /// The tick a side may first use its Glyph of Fortification.
    pub const fn glyph_first_use(self) -> u32 {
        self.horn + GLYPH_FIRST_USE_AFTER_HORN
    }

    /// The tick the scroll every hero starts with is first ready.
    pub const fn scroll_first_ready(self) -> u32 {
        self.horn + SCROLL_FIRST_READY_AFTER_HORN
    }
}
/// Ticks a daytime or a nighttime lasts: five minutes each, so a whole day
/// is ten (Liquipedia, Time of Day, since 7.20). The pregame is night and the
/// first day breaks at the horn (since 7.28).
pub const HALF_DAY_TICKS: u32 = 5 * 60 * TICKS_PER_SECOND;
/// Ticks between creep waves.
pub const WAVE_PERIOD_TICKS: u32 = 900;
/// Melee creeps in a wave before the count grows.
pub const MELEE_PER_WAVE: u32 = 3;
/// Ranged creeps in a wave before the count grows.
pub const RANGED_PER_WAVE: u32 = 1;
/// Waves from which the melee count takes each value.
pub const MELEE_GROWTH: [(u32, u32); 3] = [(31, 4), (61, 5), (91, 6)];
/// Wave from which a second ranged creep joins.
pub const RANGED_GROWTH_WAVE: u32 = 81;
/// First wave to bring a siege creep.
pub const FIRST_SIEGE_WAVE: u32 = 11;
/// Waves between siege creeps after the first.
pub const SIEGE_WAVE_PERIOD: u32 = 10;
/// Wave from which a second siege creep joins.
pub const SIEGE_GROWTH_WAVE: u32 = 71;
/// First wave to carry a flagbearer.
pub const FIRST_FLAGBEARER_WAVE: u32 = 5;
/// Waves between flagbearers after the first.
pub const FLAGBEARER_WAVE_PERIOD: u32 = 2;
/// Waves between creep upgrades; fifteen waves is seven and a half minutes.
pub const WAVE_UPGRADE_PERIOD: u32 = 15;
/// Upgrades a wave may carry at most.
pub const WAVE_UPGRADE_CAP: u32 = 30;
/// Health one upgrade adds to a melee creep.
pub const MELEE_UPGRADE_HP: i32 = 12;
/// Attack damage one upgrade adds to a melee creep.
pub const MELEE_UPGRADE_DAMAGE: i32 = 1;
/// Gold bounty one upgrade adds to a melee creep.
pub const MELEE_UPGRADE_GOLD: i32 = 1;
/// Health one upgrade adds to a ranged creep.
pub const RANGED_UPGRADE_HP: i32 = 12;
/// Attack damage one upgrade adds to a ranged creep.
pub const RANGED_UPGRADE_DAMAGE: i32 = 2;
/// Gold bounty one upgrade adds to a ranged creep.
pub const RANGED_UPGRADE_GOLD: i32 = 3;
/// Experience one upgrade adds to a ranged creep.
pub const RANGED_UPGRADE_XP: i32 = 8;
/// World units between neighbours in a wave's rank: two melee bodies and
/// the margin.
pub const WAVE_SPAWN_SPACING: i32 = 2 * MELEE_CREEP_COLLISION + STEER_MARGIN;
/// World units the ranged rank trails the front one.
pub const WAVE_SPAWN_RANK: i32 = 96;
/// Flagbearer magic resistance, percent.
pub const FLAGBEARER_MAGIC_RESIST_PCT: i32 = 40;
/// Flagbearer experience.
pub const FLAGBEARER_XP: i32 = 60;
/// Gold a flagbearer's fall pays on top of its own bounty, before the share
/// of that bounty [`FLAGBEARER_BONUS_BOUNTY_PCT`] adds.
pub const FLAGBEARER_BONUS_GOLD: i32 = 10;
/// Share of a flagbearer's own bounty, drawn when it spawns, that its fall
/// pays on top, percent.
pub const FLAGBEARER_BONUS_BOUNTY_PCT: i32 = 100;
/// How far from a fallen flagbearer an enemy hero stands to be paid its
/// bonus.
pub const FLAGBEARER_BONUS_RADIUS: i32 = 1500;

// What the three attributes are worth to whatever holds attributes.

/// Health one point of strength adds.
pub const HP_PER_STRENGTH: i32 = 22;
/// Health per tick one point of strength mends.
pub const HP_REGEN_PER_STRENGTH: Fixed = Fixed::from_ratio(1, 10 * TICKS_PER_SECOND as i32);
/// Armor one point of agility adds.
pub const ARMOR_PER_AGILITY: Fixed = Fixed::from_ratio(1, 6);
/// Attack speed one point of agility adds.
pub const ATTACK_SPEED_PER_AGILITY: i32 = 1;
/// Mana one point of intelligence adds.
pub const MANA_PER_INTELLIGENCE: i32 = 12;
/// Mana per tick one point of intelligence mends.
pub const MANA_REGEN_PER_INTELLIGENCE: Fixed = Fixed::from_ratio(1, 20 * TICKS_PER_SECOND as i32);
/// Attack damage one point of the primary attribute adds.
pub const DAMAGE_PER_PRIMARY: i32 = 1;
/// Magic resistance one point of intelligence adds, in basis points: 0.1%,
/// added to the unit's base resistance before anything that multiplies it
/// (Dota 7.41, "Each point of Intelligence grants ... 0.1% Magic
/// Resistance"; its stats tooltip calls the sum Base Magic Resistance).
pub const MAGIC_RESIST_BP_PER_INTELLIGENCE: i32 = 10;

// Attack speed. A unit swings at its own pace at [`BASE_ATTACK_SPEED`], and
// twice that pace at twice the number.

/// Attack speed with nothing added to it.
pub const BASE_ATTACK_SPEED: i32 = 100;
/// Slowest a unit may be brought to swing.
pub const MIN_ATTACK_SPEED: i32 = 20;
/// Fastest a unit may be brought to swing.
pub const MAX_ATTACK_SPEED: i32 = 700;

// Hero stats: what a hero keeps where it names none of its own.

/// Hero health at level one, before strength.
pub const HERO_HP: i32 = 180;
/// Hero mana at level one, before intelligence.
pub const HERO_MANA: i32 = 84;
/// Hero movement speed, world units per second.
pub const HERO_MOVE_SPEED: i32 = 300;
/// The least a unit that moves at all moves, world units per second, however
/// it is slowed (Dota's movement speed limits: 100 to 550).
pub const MIN_MOVE_SPEED: i32 = 100;
/// The most a unit moves, world units per second, however it is hastened.
pub const MAX_MOVE_SPEED: i32 = 550;
/// Hero attack damage at level one, before the primary attribute.
pub const HERO_ATTACK_DAMAGE: i32 = 31;
pub const HERO_ATTACK_RANGE: i32 = 600;
/// Milliseconds between hero attack starts at [`BASE_ATTACK_SPEED`].
pub const HERO_ATTACK_TIME: u32 = 2100;
/// Milliseconds from attack start to the projectile leaving.
pub const HERO_ATTACK_POINT: u32 = 300;
/// Hero attack projectile speed, world units per second.
pub const HERO_PROJECTILE_SPEED: i32 = 900;
/// Hero armor, before agility.
pub const HERO_ARMOR: i32 = -1;
/// Hero magic resistance, percent.
pub const HERO_MAGIC_RESIST_PCT: i32 = 25;
/// Hero collision size, the Dota hero hull.
pub const HERO_COLLISION: i32 = 27;
/// Hero bound radius, the Dota hero hull.
pub const HERO_BOUND: i32 = 24;
/// Hero fog light radius by day: `npc_dota_hero_base`'s
/// `VisionDaytimeRange` (client 6957).
pub const HERO_DAY_VISION: i32 = 1800;
/// Hero fog light radius by night: its `VisionNighttimeRange`.
pub const HERO_NIGHT_VISION: i32 = 800;
/// Hero attributes at level one.
pub const HERO_ATTRIBUTES: Attributes = Attributes {
    strength: Fixed::from_int(20),
    agility: Fixed::from_int(24),
    intelligence: Fixed::from_int(18),
};
/// Attributes a hero gains per level past the first.
pub const HERO_ATTRIBUTES_PER_LEVEL: Attributes = Attributes {
    strength: Fixed::from_ratio(20, 10),
    agility: Fixed::from_ratio(28, 10),
    intelligence: Fixed::from_ratio(18, 10),
};
/// Extra health per level past the first, before strength.
pub const HERO_HP_PER_LEVEL: i32 = 46;
/// Extra mana per level past the first, before intelligence.
pub const HERO_MANA_PER_LEVEL: i32 = 8;
/// Extra attack damage per level past the first, before the primary attribute.
pub const HERO_ATTACK_DAMAGE_PER_LEVEL: i32 = 3;
/// Health per tick a hero mends before strength.
pub const HERO_HP_REGEN: Fixed = Fixed::from_ratio(1, 4 * TICKS_PER_SECOND as i32);
/// Mana per tick a hero mends before intelligence.
pub const HERO_MANA_REGEN: Fixed = Fixed::from_ratio(1, 4 * TICKS_PER_SECOND as i32);
/// Highest hero level.
pub const HERO_MAX_LEVEL: u8 = 30;
/// Total experience required to sit at each level, indexed by `level - 1`.
pub const XP_THRESHOLDS: [i32; HERO_MAX_LEVEL as usize] = [
    0, 240, 640, 1160, 1760, 2440, 3200, 4000, 4900, 5900, 7000, 8200, 9500, 10900, 12400, 14000,
    15700, 17500, 19400, 21400, 23600, 26000, 28600, 31400, 34400, 38400, 43400, 49400, 56400,
    63900,
];
/// Seconds a dead hero waits before respawning, indexed by `level - 1`.
/// Past the last entry the wait is the last entry's (Dota 7.41f, Liquipedia
/// "Death": 12 s at level 1 up to 100 s from level 25).
pub const RESPAWN_SECONDS: [u32; 25] = [
    12, 15, 18, 21, 24, 26, 28, 30, 32, 34, 36, 44, 46, 48, 50, 52, 54, 65, 70, 75, 80, 85, 90, 95,
    100,
];
/// The least a hero waits once neutral creeps brought it down, in seconds
/// (Dota: "dying to neutral creeps sets respawn time to a minimum of 26s",
/// Liquipedia "Death").
pub const NEUTRAL_KILL_RESPAWN_SECONDS: u32 = 26;

// Creep stats: melee, ranged, siege.

/// Melee creep health.
pub const MELEE_CREEP_HP: i32 = 550;
/// Least attack damage of a melee creep's swing, of 19 to 23.
pub const MELEE_CREEP_ATTACK_DAMAGE: i32 = 19;
/// How much more a melee creep's swing may deal.
pub const MELEE_CREEP_DAMAGE_SPREAD: i32 = 4;
pub const MELEE_CREEP_ATTACK_RANGE: i32 = 100;
/// How far a melee creep looks for something to attack.
pub const MELEE_CREEP_ACQUISITION: i32 = 500;
pub const MELEE_CREEP_ARMOR: i32 = 2;
/// Melee creep collision size, the Dota regular hull.
pub const MELEE_CREEP_COLLISION: i32 = 36;
/// Melee creep bound radius, the Dota regular hull.
pub const MELEE_CREEP_BOUND: i32 = 16;
/// Milliseconds from a melee creep's attack start to the hit.
pub const MELEE_CREEP_ATTACK_POINT: u32 = 466;
/// Milliseconds a melee creep recovers after the hit (Liquipedia, *Attack
/// Animation*: 0.467 + 0.533).
pub const MELEE_CREEP_ATTACK_BACKSWING: u32 = 533;
pub const MELEE_CREEP_BOUNTY: GoldRange = GoldRange::new(34, 39);
/// Melee creep experience.
pub const MELEE_CREEP_XP: i32 = 57;
/// Health per tick a melee creep mends; half a point a second.
pub const MELEE_CREEP_HP_REGEN: Fixed = Fixed::from_ratio(1, 2 * TICKS_PER_SECOND as i32);

/// Ranged creep health.
pub const RANGED_CREEP_HP: i32 = 300;
/// Ranged creep armor: none (`ArmorPhysical` 0 in `npc_units.txt`).
pub const RANGED_CREEP_ARMOR: i32 = 0;
/// Least attack damage of a ranged creep's swing, of 21 to 26.
pub const RANGED_CREEP_ATTACK_DAMAGE: i32 = 21;
/// How much more a ranged creep's swing may deal.
pub const RANGED_CREEP_DAMAGE_SPREAD: i32 = 5;
pub const RANGED_CREEP_ATTACK_RANGE: i32 = 500;
/// How far a ranged creep looks for something to attack.
pub const RANGED_CREEP_ACQUISITION: i32 = 600;
/// Ranged creep collision size, the Dota small hull.
pub const RANGED_CREEP_COLLISION: i32 = 18;
/// Ranged creep bound radius, the Dota small hull.
pub const RANGED_CREEP_BOUND: i32 = 8;
/// Milliseconds from a ranged creep's attack start to the projectile
/// leaving.
pub const RANGED_CREEP_ATTACK_POINT: u32 = 500;
/// Milliseconds a ranged creep recovers after the missile leaves
/// (Liquipedia, *Attack Animation*: 0.5 + 0.3).
pub const RANGED_CREEP_ATTACK_BACKSWING: u32 = 300;
/// Ranged creep attack projectile speed, world units per second.
pub const RANGED_CREEP_PROJECTILE_SPEED: i32 = 900;
pub const RANGED_CREEP_BOUNTY: GoldRange = GoldRange::new(43, 52);
/// Ranged creep experience.
pub const RANGED_CREEP_XP: i32 = 69;
/// Health per tick a ranged creep mends; two points a second.
pub const RANGED_CREEP_HP_REGEN: Fixed = Fixed::from_ratio(2, TICKS_PER_SECOND as i32);

/// Siege creep health.
pub const SIEGE_CREEP_HP: i32 = 935;
/// Least attack damage of a siege creep's swing, of 35 to 46.
pub const SIEGE_CREEP_ATTACK_DAMAGE: i32 = 35;
/// How much more a siege creep's swing may deal.
pub const SIEGE_CREEP_DAMAGE_SPREAD: i32 = 11;
pub const SIEGE_CREEP_ATTACK_RANGE: i32 = 690;
/// How far a siege creep looks for something to attack.
pub const SIEGE_CREEP_ACQUISITION: i32 = 800;
pub const SIEGE_CREEP_ARMOR: i32 = 0;
/// Siege creep magic resistance, percent.
pub const SIEGE_CREEP_MAGIC_RESIST_PCT: i32 = 80;
/// Siege creep collision size, the Dota siege hull.
pub const SIEGE_CREEP_COLLISION: i32 = 40;
/// Siege creep bound radius, the Dota siege hull.
pub const SIEGE_CREEP_BOUND: i32 = 16;
/// Milliseconds between siege creep attack starts.
pub const SIEGE_CREEP_ATTACK_TIME: u32 = 3000;
/// Milliseconds from a siege creep's attack start to the projectile
/// leaving.
pub const SIEGE_CREEP_ATTACK_POINT: u32 = 700;
/// Milliseconds a siege creep recovers after the missile leaves
/// (Liquipedia, *Attack Animation*: 0.7 + 1.3).
pub const SIEGE_CREEP_ATTACK_BACKSWING: u32 = 1300;
/// Siege creep attack projectile speed, world units per second.
pub const SIEGE_CREEP_PROJECTILE_SPEED: i32 = 1100;
pub const SIEGE_CREEP_BOUNTY: GoldRange = GoldRange::new(59, 72);
/// Siege creep experience.
pub const SIEGE_CREEP_XP: i32 = 88;

/// Creep movement speed, world units per second.
pub const CREEP_MOVE_SPEED: i32 = 325;
/// Milliseconds between creep attack starts.
pub const CREEP_ATTACK_TIME: u32 = 1000;
/// Lane creep fog light radius, by day and by night alike.
pub const CREEP_VISION: i32 = 750;

// Super and mega creeps: what a wave grows into once the enemy barracks that
// held it back have fallen.

/// Super melee creep health.
pub const SUPER_MELEE_HP: i32 = 700;
/// Least attack damage of a super melee creep's swing, of 36 to 44.
pub const SUPER_MELEE_ATTACK_DAMAGE: i32 = 36;
/// How much more a super melee creep's swing may deal.
pub const SUPER_MELEE_DAMAGE_SPREAD: i32 = 8;
/// Super melee creep armor.
pub const SUPER_MELEE_ARMOR: i32 = 3;
/// Super melee creep gold bounty.
pub const SUPER_MELEE_BOUNTY: GoldRange = GoldRange::new(20, 26);
/// Super melee creep experience.
pub const SUPER_MELEE_XP: i32 = 25;
/// Super ranged creep health.
pub const SUPER_RANGED_HP: i32 = 475;
/// Least attack damage of a super ranged creep's swing, of 41 to 46.
pub const SUPER_RANGED_ATTACK_DAMAGE: i32 = 41;
/// How much more a super ranged creep's swing may deal.
pub const SUPER_RANGED_DAMAGE_SPREAD: i32 = 5;
/// Super ranged creep armor.
pub const SUPER_RANGED_ARMOR: i32 = 1;
/// Super ranged creep gold bounty.
pub const SUPER_RANGED_BOUNTY: GoldRange = GoldRange::new(19, 25);
/// Super ranged creep experience.
pub const SUPER_RANGED_XP: i32 = 22;
/// Least attack damage of a super siege creep's swing, of 51 to 62.
pub const SUPER_SIEGE_ATTACK_DAMAGE: i32 = 51;
/// How much more a super siege creep's swing may deal.
pub const SUPER_SIEGE_DAMAGE_SPREAD: i32 = 11;
/// Health one upgrade adds to a super melee creep.
pub const SUPER_MELEE_UPGRADE_HP: i32 = 19;
/// Attack damage one upgrade adds to a super melee creep.
pub const SUPER_MELEE_UPGRADE_DAMAGE: i32 = 2;
/// Half gold one upgrade adds to a super or mega melee creep's bounty.
pub const SUPER_MELEE_UPGRADE_GOLD_HALVES: i32 = 3;
/// Health one upgrade adds to a super ranged creep.
pub const SUPER_RANGED_UPGRADE_HP: i32 = 18;
/// Attack damage one upgrade adds to a super ranged creep.
pub const SUPER_RANGED_UPGRADE_DAMAGE: i32 = 3;
/// Gold one upgrade adds to a super or mega ranged creep's bounty.
pub const SUPER_RANGED_UPGRADE_GOLD: i32 = 6;
/// Mega melee creep health.
pub const MEGA_MELEE_HP: i32 = 1270;
/// Least attack damage of a mega melee creep's swing, of 96 to 104.
pub const MEGA_MELEE_ATTACK_DAMAGE: i32 = 96;
/// Mega ranged creep health.
pub const MEGA_RANGED_HP: i32 = 1015;
/// Least attack damage of a mega ranged creep's swing, of 131 to 136.
pub const MEGA_RANGED_ATTACK_DAMAGE: i32 = 131;

// Buildings.

/// Tower health, indexed by tier minus one.
pub const TOWER_TIER_HP: [i32; 4] = [1800, 2500, 2500, 2600];
/// Least attack damage of a tower's shot, indexed by tier minus one: 88 to
/// 92 at tier one, 170 to 174 above it.
pub const TOWER_TIER_DAMAGE: [i32; 4] = [88, 170, 170, 170];
/// How much more a tower's shot may deal.
pub const TOWER_DAMAGE_SPREAD: i32 = 4;
/// Tower armor, indexed by tier minus one.
pub const TOWER_TIER_ARMOR: [i32; 4] = [12, 16, 16, 21];
/// A tower's attack speed.
pub const TOWER_ATTACK_SPEED: i32 = 110;
/// How fast a tower turns, in brads per tick: a `MovementTurnRate` of 1.
pub const TOWER_TURN_RATE_BRADS: u16 = 11_590;
/// Gold paid to the hero that strikes a tower down, on top of its team
/// bounty, indexed by tier minus one.
pub const TOWER_TIER_BOUNTY: [GoldRange; 4] = [
    GoldRange::new(110, 130),
    GoldRange::new(130, 150),
    GoldRange::new(150, 170),
    GoldRange::new(170, 190),
];
/// Gold a tower's fall pays every seat of the side that brought it down,
/// indexed by tier minus one; a deny pays half of it to every seat of both
/// sides.
pub const TOWER_TIER_TEAM_BOUNTY: [i32; 4] = [90, 110, 125, 145];
pub const TOWER_ATTACK_RANGE: i32 = 700;
/// Milliseconds between tower attack starts at [`BASE_ATTACK_SPEED`].
pub const TOWER_ATTACK_TIME: u32 = 900;
/// Milliseconds from tower attack start to the projectile leaving, at
/// [`BASE_ATTACK_SPEED`].
pub const TOWER_ATTACK_POINT: u32 = 600;
/// Tower attack projectile speed, world units per second.
pub const TOWER_PROJECTILE_SPEED: i32 = 750;
/// How far a tower's protection reaches.
pub const TOWER_AURA_RADIUS: i32 = 900;
/// Armor a tower's protection adds, in whole points, indexed by tier less
/// one.
pub const TOWER_AURA_ARMOR: [i32; 4] = [3, 5, 5, 5];
/// Health a tower's protection mends, in hundredths of a point a second,
/// indexed by tier less one.
pub const TOWER_AURA_REGEN: [i32; 4] = [100, 300, 300, 300];
/// Ticks an aura's modifier holds once handed out, which is how long it
/// lingers after walking out of one: Dota's half second, the fountain's
/// excepted.
pub const AURA_LINGER_TICKS: u32 = TICKS_PER_SECOND / 2;
/// How far a flagbearer's inspiration reaches.
pub const FLAGBEARER_AURA_RADIUS: i32 = 700;
/// Health a flagbearer's inspiration mends, in hundredths of a point a
/// second.
pub const FLAGBEARER_AURA_REGEN: i32 = 300;
/// Magic resistance a flagbearer's inspiration grants for each upgrade
/// interval the flagbearer spawned after, percent.
pub const FLAGBEARER_AURA_MAGIC_RESIST_PCT: i32 = 4;
/// Upgrade intervals the inspiration's magic resistance grows for at most.
pub const FLAGBEARER_AURA_MAGIC_RESIST_CAP: u32 = 15;
/// Tower collision size, the Dota tower hull.
pub const TOWER_COLLISION: i32 = 144;
/// Tower bound radius, the Dota tower hull.
pub const TOWER_BOUND: i32 = 144;
/// Tower fog light radius by day.
pub const TOWER_DAY_VISION: i32 = 1900;
/// Tower fog light radius by night, by tier less one: a tier-one tower sees
/// 800, the towers behind it 1100 (`npc_units.txt`, client 6957).
pub const TOWER_NIGHT_VISION: [i32; 4] = [800, 1100, 1100, 1100];

/// Percent of its damage a structure under backdoor protection takes.
pub const BACKDOOR_DAMAGE_PCT: i32 = 50;
/// Health a structure under backdoor protection mends a second, but only of
/// what enemy blows took from it while it was protected.
pub const BACKDOOR_HEAL_PER_SECOND: i32 = 180;
/// Ticks without an enemy lane creep in reach before backdoor protection
/// holds again: fifteen seconds and the half second the aura lingers.
pub const BACKDOOR_REACTIVATION_TICKS: u32 = 15 * TICKS_PER_SECOND + AURA_LINGER_TICKS;
/// How far a tier two tower looks for enemy lane creeps; it guards itself.
pub const BACKDOOR_TOWER_RADIUS: i32 = 900;
/// How far an Ancient looks for enemy lane creeps; it guards every tower of
/// tier three and four, barracks and filler of its side, and itself.
pub const BACKDOOR_BASE_RADIUS: i32 = 4000;

/// Ticks from the horn until a side may first use its Glyph of
/// Fortification. Dota puts it on its cooldown from the very first second
/// of the match, pregame included, showing 4:30 then (checked in the Dota 2
/// client, 7.41, 2026-10-24), so it cannot be pressed before then: 270
/// seconds from the heroes coming in, which is three minutes past the horn
/// (Liquipedia, *Glyph of Fortification*: "cannot be used until the 3:00
/// minute mark"). Dota's heroes come in [`DOTA_PREGAME_TICKS`] before the
/// horn; a map with a shorter pregame lays the wait on the game clock, as
/// the first scroll's is.
pub const GLYPH_FIRST_USE_AFTER_HORN: u32 = 270 * TICKS_PER_SECOND - DOTA_PREGAME_TICKS;
/// Cooldown of a Glyph of Fortification, shared by a whole side: 300 seconds.
pub const GLYPH_COOLDOWN_TICKS: u32 = 300 * TICKS_PER_SECOND;
/// How long a Glyph keeps a side's structures from taking damage: 7 seconds
/// (`abilities_english.txt`, `fountain_glyph_duration_structures`).
pub const GLYPH_STRUCTURE_TICKS: u32 = 7 * TICKS_PER_SECOND;
/// How long a Glyph keeps a side's lane creeps from taking damage: 3
/// seconds (`fountain_glyph_duration_creeps`).
pub const GLYPH_CREEP_TICKS: u32 = 3 * TICKS_PER_SECOND;
/// Extra targets a tower of tier two and above shoots at with each shot
/// while its side's Glyph holds (`fountain_glyph_multishot_targets`).
pub const GLYPH_MULTISHOT_TARGETS: usize = 5;
/// Ticks after a side's first tier one tower, first tier two tower and first
/// melee barracks fall before its Glyph is ready again: the cooldown is set
/// to one second.
pub const GLYPH_REFRESH_TICKS: u32 = TICKS_PER_SECOND;

/// Melee barracks health.
pub const RAX_MELEE_HP: i32 = 2200;
/// Melee barracks armor.
pub const RAX_MELEE_ARMOR: i32 = 15;
/// Health per tick a melee barracks mends; five a second.
pub const RAX_MELEE_HP_REGEN: Fixed = Fixed::from_ratio(5, TICKS_PER_SECOND as i32);
/// Ranged barracks health.
pub const RAX_RANGED_HP: i32 = 1300;
/// Ranged barracks armor.
pub const RAX_RANGED_ARMOR: i32 = 9;
/// Gold paid to the hero that strikes a barracks down, on top of its team
/// bounty.
pub const RAX_BOUNTY: GoldRange = GoldRange::new(90, 135);
/// Gold a melee barracks' fall pays every seat of the side that brought it
/// down.
pub const RAX_MELEE_TEAM_BOUNTY: i32 = 155;
/// Gold a ranged barracks' fall pays every seat of the side that brought it
/// down.
pub const RAX_RANGED_TEAM_BOUNTY: i32 = 90;
/// Barracks collision size, the Dota barracks hull.
pub const RAX_COLLISION: i32 = 160;
/// Barracks bound radius, the Dota barracks hull.
pub const RAX_BOUND: i32 = 144;
/// Barracks fog light radius by day.
pub const RAX_DAY_VISION: i32 = 900;
/// Barracks fog light radius by night.
pub const RAX_NIGHT_VISION: i32 = 600;

/// Filler health.
pub const FILLER_HP: i32 = 1000;
pub const FILLER_ARMOR: i32 = 10;
/// Gold paid to the killer of a filler.
pub const FILLER_BOUNTY: i32 = 68;
/// Filler collision size, the Dota filler hull.
pub const FILLER_COLLISION: i32 = 112;
/// Filler bound radius, the Dota filler hull.
pub const FILLER_BOUND: i32 = 96;
/// Filler fog light radius by day.
pub const FILLER_DAY_VISION: i32 = 900;
/// Filler fog light radius by night.
pub const FILLER_NIGHT_VISION: i32 = 600;

/// Ancient health.
pub const ANCIENT_HP: i32 = 4500;
/// Health per tick the Ancient mends; twelve a second.
pub const ANCIENT_HP_REGEN: Fixed = Fixed::from_ratio(12, TICKS_PER_SECOND as i32);
/// Ancient armor (`npc_dota_goodguys_fort`).
pub const ANCIENT_ARMOR: i32 = 23;
/// The Radiant Ancient's collision size, its own model's.
pub const RADIANT_ANCIENT_COLLISION: i32 = 315;
/// The Radiant Ancient's bound radius, its own model's.
pub const RADIANT_ANCIENT_BOUND: i32 = 299;
/// The Dire Ancient's collision size, its own model's.
pub const DIRE_ANCIENT_COLLISION: i32 = 390;
/// The Dire Ancient's bound radius, its own model's.
pub const DIRE_ANCIENT_BOUND: i32 = 374;
/// Ancient fog light radius, by day and by night alike.
pub const ANCIENT_VISION: i32 = 2600;

/// Fountain health. It is never lost: the fountain is invulnerable.
pub const FOUNTAIN_HP: i32 = 500;
/// Fountain attack damage, the midpoint of `dota_fountain`'s 290 to 310
/// (`npc_units.txt`, 7.41f).
pub const FOUNTAIN_ATTACK_DAMAGE: i32 = 300;
/// Fountain attack and acquisition range: `dota_fountain`'s 1200.
pub const FOUNTAIN_ATTACK_RANGE: i32 = 1200;
/// Milliseconds between fountain attack starts: `dota_fountain`'s
/// `AttackRate` 0.15 at attack speed 100, 6.67 shots a second.
pub const FOUNTAIN_ATTACK_TIME: u32 = 150;
/// Milliseconds from fountain attack start to the shot: none, as
/// `dota_fountain`'s `AttackAnimationPoint` 0.0.
pub const FOUNTAIN_ATTACK_POINT: u32 = 0;
/// Fountain attack projectile speed, world units per second.
pub const FOUNTAIN_PROJECTILE_SPEED: i32 = 1400;
/// How far the fountain reveals what hides: its True Sight (Liquipedia,
/// *Fountain*: "Reveal Radius: 1200").
pub const FOUNTAIN_TRUE_SIGHT: i32 = 1200;
/// What each stack of the fountain's Fountain Damage adds to its shots
/// (Liquipedia, *Fountain*: "Attack Damage Bonus per Stack: 3"; no most
/// stacks). The stack is added before the shot deals its damage.
pub const FOUNTAIN_STACK_DAMAGE: i32 = 3;
/// How long the fountain's stacks hold on a unit after its last shot that
/// landed (Liquipedia, *Fountain*: "Debuff Duration: 12").
pub const FOUNTAIN_STACK_TICKS: u32 = 12 * TICKS_PER_SECOND;
/// How far round what a fountain's shot lands on it splashes (Liquipedia,
/// *Fountain*: "Splash Radius: 250").
pub const FOUNTAIN_SPLASH_RADIUS: i32 = 250;
/// The share of a fountain's shot each other enemy in its splash takes, as
/// physical damage (Liquipedia, *Fountain*: "Splash Damage: 25%").
pub const FOUNTAIN_SPLASH_PCT: i32 = 25;
/// The chance a fountain's shot goes through evasion (Liquipedia,
/// *Fountain*: "Accuracy: 25%").
pub const FOUNTAIN_ACCURACY_PCT: u8 = 25;
/// Fountain collision size, the Dota tower hull.
pub const FOUNTAIN_COLLISION: i32 = 144;
/// Fountain bound radius, the Dota tower hull.
pub const FOUNTAIN_BOUND: i32 = 144;
/// Fountain fog light radius, by day and by night alike.
pub const FOUNTAIN_VISION: i32 = 1800;
/// Percent of its maximum health the fountain mends a second on its own
/// side: Dota 7.41f's Rejuvenation Aura, full in twenty seconds
/// (Liquipedia, *Fountain*, read 2026-10).
pub const FOUNTAIN_HEAL_HP_PCT: i32 = 5;
/// Percent of its maximum mana the fountain mends a second on its own side,
/// full in sixteen and two thirds.
pub const FOUNTAIN_HEAL_MANA_PCT: i32 = 6;
/// Ticks the fountain's mending lingers on a unit that steps out: the
/// aura's three seconds.
pub const FOUNTAIN_HEAL_LINGER_TICKS: u32 = 3 * TICKS_PER_SECOND;
/// Radius of the fountain heal area.
pub const FOUNTAIN_HEAL_RADIUS: i32 = 1200;

// Combat.

/// How far past attack range a started attack still connects.
pub const ATTACK_RANGE_LEEWAY: i32 = 100;
/// How far an idle hero looks for something to attack.
pub const ACQUISITION_RANGE: i32 = 600;
/// Candidates within this many world units of each other rank as equally
/// close, and what a hero is doing breaks the tie.
pub const AGGRO_TIE_RANGE: i32 = 100;
/// Ticks before an attack order may re-aim the same creep or tower again,
/// 3 seconds.
pub const ORDER_AGGRO_COOLDOWN_TICKS: u32 = 90;
/// Ticks before an order at an ally may take a tower off the orderer again,
/// 2.5 seconds.
pub const TOWER_RELEASE_COOLDOWN_TICKS: u32 = 75;
/// Ticks a creep handed a target by an attack order keeps it before the
/// ordinary ranking may take it back, 2.3 seconds.
pub const ORDER_AGGRO_HOLD_TICKS: u32 = 69;
/// How near a hero must stand to a lane creep for its attack order or its
/// swing at a hero to draw the creep.
pub const HERO_AGGRO_RANGE: i32 = 500;
/// Ticks from the first wave until an attack order may draw any lane creep.
pub const FREE_AGGRO_AFTER_FIRST_WAVE: u32 = 5 * 60 * TICKS_PER_SECOND;
/// How close to its own tier-one tower a lane creep may be aggroed before
/// [`Schedule::free_aggro`].
pub const EARLY_AGGRO_TOWER_RANGE: i32 = 1500;
/// A lane creep stops chasing a target that turns up further than this from
/// where it stood a tick before: a teleport, not a walk (Liquipedia, *Lane
/// Creeps*).
pub const CREEP_CHASE_LEAP: i32 = 1600;
/// Radius of the sight an attack or a spell at a unit shows the side struck,
/// around where the attacker stood.
pub const STRIKE_REVEAL_RADIUS: i32 = 150;
/// Ticks that sight lasts, two seconds.
pub const STRIKE_REVEAL_TICKS: u32 = 2 * TICKS_PER_SECOND;
/// How near a spell's target must stand to its caster for the cast to show
/// the caster.
pub const CAST_REVEAL_RANGE: i32 = 1800;
/// Room a route keeps past the walker's own collision size, in world units.
pub const STEER_MARGIN: i32 = 8;
/// The collision size a lane route keeps clear of every obstacle: the widest
/// marcher's, so every creep of a wave can walk it.
pub const WIDEST_MARCHER: i32 = SIEGE_CREEP_COLLISION;
/// A route corner counts as reached within this distance.
pub const WAYPOINT_RADIUS: i32 = 40;
/// A stored route is laid again once its goal drifted this far.
pub const REPATH_DRIFT: i32 = 128;

/// How far a body may be eased out of another one in a tick, in world
/// units.
pub const SEPARATION_STEP: i32 = 4;
/// Milliseconds a hero recovers after a swing. Cancelled by any order.
pub const HERO_ATTACK_BACKSWING: u32 = 400;
/// Milliseconds a tower recovers after a shot, at [`BASE_ATTACK_SPEED`]:
/// the rest of its cycle, the next shot cutting the 0.4 s animation short.
pub const TOWER_ATTACK_BACKSWING: u32 = TOWER_ATTACK_TIME - TOWER_ATTACK_POINT;
/// Milliseconds the fountain recovers after a shot: none (Liquipedia,
/// *Fountain*: attack animation 0+0); its cadence is its attack time alone.
pub const FOUNTAIN_ATTACK_BACKSWING: u32 = 0;
/// How fast a unit turns, in brads per tick: a `MovementTurnRate` of 0.5,
/// read as radians per 0.03 seconds.
pub const TURN_RATE_BRADS: u16 = 5795;
/// How fast a quick unit turns, in brads per tick: a `MovementTurnRate` of
/// 0.9, read the same way.
pub const FAST_TURN_RATE_BRADS: u16 = 10431;
/// A unit starts a swing only when facing within this error of its target,
/// in brads: 11.5 degrees.
pub const ATTACK_ANGLE_BRADS: u16 = 2094;

/// A unit walks only when facing within this error of where it is going, in
/// brads.
pub const TURN_TOLERANCE_BRADS: u16 = 8192;

// Abilities.

/// Levels a basic ability can reach.
pub const ABILITY_MAX_LEVEL: u8 = 4;
/// Levels the ultimate can reach.
pub const ULT_MAX_LEVEL: u8 = 3;
/// Hero level required for each ultimate level.
pub const ULT_LEVEL_FLOORS: [u8; 3] = [6, 12, 18];
/// Hero level each tier of talents opens at, one talent of the two.
pub const TALENT_LEVELS: [u8; 4] = [10, 15, 20, 25];
/// Hero level the other talent of each tier opens at.
pub const TALENT_SECOND_LEVELS: [u8; 4] = [27, 28, 29, 30];
/// Levels the Attribute Bonus can reach: `special_bonus_attributes`
/// `MaxLevel 7` (`scripts/npc/npc_abilities.txt`, client 6957).
pub const ATTRIBUTE_BONUS_MAX_LEVEL: u8 = 7;
/// Attribute Bonus level `k` waits for hero level this many times `k`: 2, 4,
/// ... 14 (`RequiredLevel 2`; Liquipedia, *Attributes*, "the min levels
/// required per level of Attribute Bonus are level 2/4/6/8/10/12/14"; the
/// 7.38 notes moved it from 6 to 2).
pub const ATTRIBUTE_BONUS_LEVELS_APART: u8 = 2;
/// Points of every attribute a level of the Attribute Bonus adds: `value 2
/// 4 6 ... 14`.
pub const ATTRIBUTE_BONUS_POINTS: i32 = 2;

const _: () = {
    assert!(TALENT_LEVELS.len() * 2 == bota_proto::TalentSlot::COUNT as usize);
    assert!(TALENT_SECOND_LEVELS[3] <= HERO_MAX_LEVEL);
    assert!(TALENT_LEVELS[3] < TALENT_SECOND_LEVELS[0]);
};

/// Fog blocker nodes further apart than this belong to different walls of
/// the same named group, not to one span.
pub const FOW_BLOCKER_SPAN: i32 = 600;

// Items and the shop.

/// Ticks a felled tree takes to come back, three minutes: patch 7.29 "Tree
/// respawn interval reduced from 5 to 3 minutes", unchanged since (Liquipedia
/// *Trees*, Respawn Time).
pub const TREE_REGROW_TICKS: u32 = 3 * 60 * TICKS_PER_SECOND;
/// How near a tree's spot a unit standing keeps it from growing back, in
/// world units: it grows once the unit is out of range, so as not to shut
/// the unit in (Liquipedia *Trees*, Respawn Time: 150).
pub const TREE_REGROW_CLEAR: i32 = 150;
/// Ticks that sight outlasts the channel, whether it carried or broke off:
/// Liquipedia's one-second sight.
pub const SCROLL_SIGHT_LINGER_TICKS: u32 = TICKS_PER_SECOND;
/// A scroll aimed at nothing carries to the fountain only from at least
/// this far from it: Liquipedia's self-cast minimum distance 1500.
pub const SCROLL_FOUNTAIN_MIN_DISTANCE: i32 = 1500;
/// Dota's pregame, from the heroes coming in to the horn: ninety seconds.
pub const DOTA_PREGAME_TICKS: u32 = 90 * TICKS_PER_SECOND;
/// Ticks from the horn until the scroll every hero starts with is ready: it
/// comes with a wait of 100 seconds from the heroes coming in (Liquipedia,
/// Town Portal Scroll: initial cooldown 100), and Dota's heroes come in 90 s
/// before the horn, so it is ready ten seconds past the horn; a map with a
/// shorter pregame lays the wait on the game clock, as in Dota.
pub const SCROLL_FIRST_READY_AFTER_HORN: u32 = 100 * TICKS_PER_SECOND - DOTA_PREGAME_TICKS;
/// Steps out from where a scroll was aimed it looks for open ground, a
/// ring of eight spots to a step.
pub const SCROLL_LANDING_RINGS: i32 = 8;
/// World units between those rings.
pub const SCROLL_LANDING_STEP: i32 = 64;

const _: () = {
    assert!(100 * TICKS_PER_SECOND > DOTA_PREGAME_TICKS);
    assert!(270 * TICKS_PER_SECOND > DOTA_PREGAME_TICKS);
};

// The courier.

/// Health a courier stands up with, counted in hits: `npc_dota_courier`'s
/// StatusHealth.
pub const COURIER_HP: i32 = 6;
/// The attacks that bring a courier down: one melee hero swing, two ranged
/// hero missiles, six of a creep's or a building's (Dota 7.35: "Courier dies
/// after 1 melee hero attack, 2 ranged hero attacks or 6 creep/tower
/// attacks"; Liquipedia, *Courier*, the same in 7.41).
pub const COURIER_HITS: crate::HitCount = crate::HitCount {
    melee_hero: 1,
    ranged_hero: 2,
    other: 6,
};
/// World units a second a courier walks at its owner's first level:
/// `npc_dota_courier`'s MovementSpeed, on the ground
/// (`DOTA_UNIT_CAP_MOVE_GROUND`).
pub const COURIER_MOVE_SPEED: i32 = 290;
/// World units a second each level of its owner past the first adds:
/// Liquipedia, Courier, "Movement Speed Gain per Level: 10" (290 at level 1
/// to 580 at 30).
pub const COURIER_MOVE_SPEED_PER_LEVEL: i32 = 10;
/// A courier's pace while its fountain's Rejuvenation Aura holds on it,
/// whatever else would change it: "courier moves faster", 550, lingering
/// with the aura's buff 3 s after it leaves (checked in the Dota 2 client,
/// 7.41, 2026-10-24).
pub const COURIER_FOUNTAIN_SPEED: i32 = 550;
/// The owner's level from which a courier flies (`modifier_courier_flying`).
pub const COURIER_FLYING_LEVEL: u8 = 4;
/// The owner's level from which a courier may burst.
pub const COURIER_BURST_LEVEL: u8 = 10;
/// The owner's level from which a courier may shield itself.
pub const COURIER_SHIELD_LEVEL: u8 = 20;
/// How far a courier sees, by day and by night alike.
pub const COURIER_VISION: i32 = 200;
/// Seconds a courier waits to come back, before its owner's level: "Base
/// Respawn Time: 45" (Liquipedia, Courier; Dota 7.40).
pub const COURIER_RESPAWN_SECONDS: u32 = 45;
/// Seconds each level of its owner adds to the wait: "Respawn Time Increase
/// per Level: 5", 50 s at level 1 to 195 s at 30.
pub const COURIER_RESPAWN_SECONDS_PER_LEVEL: u32 = 5;
/// Gold a courier's fall pays every enemy hero at its owner's first level:
/// Liquipedia, Courier, "Team Bounty" 25 at level 1.
pub const COURIER_TEAM_BOUNTY: i32 = 25;
/// Gold each level of its owner past the first adds to it: "Gold Bounty
/// Increase per Level: 5", 170 at level 30.
pub const COURIER_TEAM_BOUNTY_PER_LEVEL: i32 = 5;
/// The collision size of Dota's small hull (`DOTA_HULL_SIZE_SMALL`), a
/// courier's.
pub const COURIER_COLLISION: i32 = RANGED_CREEP_COLLISION;
/// The bound radius of the small hull.
pub const COURIER_BOUND: i32 = RANGED_CREEP_BOUND;

/// Ticks a courier waits before it comes back, its owner at `level`.
pub const fn courier_respawn_ticks(level: u8) -> u32 {
    (COURIER_RESPAWN_SECONDS + COURIER_RESPAWN_SECONDS_PER_LEVEL * level as u32) * TICKS_PER_SECOND
}
/// Percent a burst adds to a courier's speed.
pub const COURIER_BURST_PCT: i32 = 50;
/// Ticks a burst lasts.
pub const COURIER_BURST_TICKS: u32 = 6 * TICKS_PER_SECOND;
/// Ticks between bursts.
pub const COURIER_BURST_COOLDOWN: u32 = 120 * TICKS_PER_SECOND;
/// Ticks a courier's shield holds.
pub const COURIER_SHIELD_TICKS: u32 = 2 * TICKS_PER_SECOND;
/// Ticks between shields.
pub const COURIER_SHIELD_COOLDOWN: u32 = 200 * TICKS_PER_SECOND;
/// How near its home fountain a courier has to be to reach the stash.
pub const COURIER_STASH_RANGE: i32 = 700;
/// How close a courier must be to hand over what it carries.
pub const COURIER_DELIVER_RANGE: i32 = 200;

/// How far ahead of itself a feared unit aims each step, in world units.
pub const FLEE_LOOKAHEAD: i32 = 600;
/// Ticks a burst or a ring an ability left is seen on the ground.
pub const MARK_TICKS: u32 = 15;

/// Ticks between the beats of anything that burns over time.
pub const BURN_PERIOD_TICKS: u32 = 3;

/// The chance a ranged attack thrown at a target on higher ground misses:
/// one more source of the target's evasion, as Dota counts it.
pub const UPHILL_MISS: crate::Odds = crate::Odds::percent(25);
/// Ticks an item stays muted after leaving the backpack for the inventory.
pub const BACKPACK_MUTE_TICKS: u32 = 180;
/// How close to the home fountain the shop, the stash and selling work.
pub const SHOP_RANGE: i32 = 1000;
/// Percent of the price a sale returns.
pub const SELL_PCT: i32 = 50;
/// Ticks after purchase in which an unused item refunds in full.
pub const SELL_REFUND_TICKS: u32 = 300;
/// Ticks after being combined in which an item whose rule is the default may be
/// taken apart again: the ten seconds Dota's items can be disassembled in.
pub const DISASSEMBLY_WINDOW_TICKS: u32 = 300;
/// How far an item may be laid on the ground or handed to another bag, in
/// world units.
pub const PUT_ITEM_RANGE: i32 = 400;
/// How close a unit must be to take an item off the ground, in world units.
pub const TAKE_ITEM_RANGE: i32 = 150;
/// How far back along its line a blink steps looking for open ground.
pub const BLINK_STEP_BACK: i32 = 64;
/// Steps back a blink takes before it gives up on finding open ground.
pub const BLINK_STEP_TRIES: u32 = 20;
/// Denominator scale of the armor formula: each point of armor adds
/// `ARMOR_SCALE` to a base of one hundred, and below zero each point lets
/// as much more through.
pub const ARMOR_SCALE: i32 = 6;

// Economy.

/// Gold each seat starts the match with.
pub const STARTING_GOLD: i32 = 600;
/// Seconds of hero selection: Dota's default (Liquipedia, *Game Modes*: All
/// Pick has 75 "instead of the default 60"; Solo Mid and Mid Only set none
/// of their own).
pub const PICK_SECS: u64 = 60;
/// Seconds past the selection time after which a seat still without a hero
/// is given a random one (Liquipedia, *Game Modes*, Ranked All Pick: "if a
/// player fails to pick a hero and loses gold for more than 30 seconds,
/// they are assigned a random hero").
pub const PICK_PENALTY_SECS: u64 = 30;
/// Seconds of tactical phase after the picks: Dota's "thirty seconds of
/// guaranteed strategy time to make final preparations and item purchases"
/// (dota2.com/700/pregame).
pub const TACTICAL_SECS: u64 = 30;
/// Gold a seat loses from its starting gold for every whole second it picks
/// after the selection time (Liquipedia, *Game Modes*, All Pick: "players
/// lose 2 gold per second if they haven't picked a hero after the selection
/// timer runs out").
pub const PICK_PENALTY_GOLD_PER_SEC: i32 = 2;
/// Passive gold a minute, in tenths of a gold, from each game minute on.
/// Dota 7.41f pays one reliable gold every 0.67 s from 0:00, 90 a minute,
/// rising with the clock (Liquipedia and the Dota 2 wiki, *Gold*, "Periodic
/// Gold", read 2026-10; no patch note from 7.33 to 7.41f changes it).
pub const PASSIVE_GOLD_TENTHS_A_MINUTE: [(u32, i64); 9] = [
    (0, 900),
    (12, 948),
    (30, 998),
    (45, 1055),
    (62, 1128),
    (87, 1205),
    (112, 1290),
    (140, 1390),
    (175, 1505),
];
/// A dying hero loses its net worth divided by this, never more gold than
/// it holds.
pub const DEATH_GOLD_LOSS_SHARE: i32 = 40;
/// Gold for killing a hero, before its level, its streak and First Blood:
/// Dota's `125 + 8 * level + streak gold + First Blood` since 7.31
/// (Liquipedia, *Gold*, "Hero kills" and "Kill Streaks", read 2026-10).
pub const HERO_KILL_GOLD_BASE: i32 = 125;
/// Gold for killing a hero, a level of the fallen.
pub const HERO_KILL_GOLD_PER_LEVEL: i32 = 8;
/// Gold for ending a streak of 3, 4, ... 10 or more kills.
pub const STREAK_GOLD: [i32; 8] = [60, 100, 150, 210, 280, 360, 450, 550];
/// The shortest streak whose end pays gold.
pub const STREAK_GOLD_FROM: i32 = 3;
/// Gold the match's first hero kill pays its killer on top.
pub const FIRST_BLOOD_GOLD: i32 = 135;
/// Assist gold every earner of a hero kill is paid before the net worth
/// factor, in thousandths: `ASSIST_GOLD_FLAT + (ASSIST_GOLD_SHARED +
/// ASSIST_GOLD_NET_WORTH * the fallen's net worth) / earners`, Dota 7.40's
/// "15 + ((50 + (VictimNetworth * 0.037)) / NumHeroes)".
pub const ASSIST_GOLD_FLAT_MILLI: i64 = 15_000;
/// The part of the assist gold the earners share, in thousandths.
pub const ASSIST_GOLD_SHARED_MILLI: i64 = 50_000;
/// Thousandths of the fallen hero's net worth the earners share.
pub const ASSIST_GOLD_NET_WORTH_MILLI: i64 = 37;
/// Experience for killing a hero, before the share of its own.
pub const HERO_KILL_XP_BASE: i32 = 100;
/// Percent of the fallen hero's own experience paid for its head besides.
pub const HERO_KILL_XP_SHARE_PCT: i32 = 13;
/// The fallen hero's experience counts up to this much: a head stops
/// growing at level 25 (Liquipedia, *Experience*).
pub const HERO_KILL_XP_COUNTED_MAX: i32 = XP_THRESHOLDS[24];
/// The shortest streak whose end pays experience.
pub const STREAK_XP_FROM: i32 = 3;
/// Streaks longer than this pay no more for their end.
pub const STREAK_XP_CAP: i32 = 10;

const _: () = assert!(STREAK_GOLD.len() as i32 == STREAK_XP_CAP - STREAK_GOLD_FROM + 1);
/// Radius around a death within which enemy heroes receive experience.
pub const XP_RADIUS: i32 = 1500;

/// A friendly lane creep may be denied when its health is strictly below
/// this percent of its maximum.
pub const DENY_HP_PCT: i32 = 50;

/// A ward's health, counted in hits: 2, a hero's attack taking 1 (checked
/// in the Dota 2 client, 7.41, 2026-10-24).
pub const WARD_HP: i32 = 2;

/// An allied ward may be attacked when its health is strictly below this
/// percent of its maximum (Dota 7.41f's ward deny range).
pub const WARD_DENY_HP_PCT: i32 = 50;

/// The attacks that bring a ward down: two of a hero's, as its health is
/// counted (the Dota 2 client), and ten of anything else's, a creep's, a
/// tower's or an illusion's, each a fifth of a hit (Liquipedia, *Observer
/// Ward*: "Hero Hit Count: 2 Non-Hero Hit Count: 10"; the Sentry Ward's the
/// same).
pub const WARD_HITS: crate::HitCount = crate::HitCount {
    melee_hero: 2,
    ranged_hero: 2,
    other: 10,
};

/// How much further a melee attacker reaches when it attacks a ward.
pub const WARD_MELEE_RANGE_BONUS: i32 = 150;

/// A friendly building may be denied when its health is strictly below
/// this percent of its maximum.
pub const DENY_BUILDING_HP_PCT: i32 = 10;
/// Percent of a denied lane creep's experience the enemy still gets.
pub const DENIED_XP_PCT: i32 = 50;

/// A whole number of world units as a [`Fixed`] distance.
#[inline]
pub const fn units(n: i32) -> Fixed {
    Fixed::from_int(n)
}
