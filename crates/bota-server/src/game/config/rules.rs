//! Balance constants.
//!
//! Everything here is in ticks, whole world units or plain integers. Time
//! constants assume [`TICKS_PER_SECOND`]; the wall-clock pace of a match is a
//! server option and does not change them.

use bota_proto::{Attributes, Fixed, Vec2};

use crate::game::Ratio;

/// Simulation ticks per second of game time.
pub const TICKS_PER_SECOND: u32 = 30;

/// The map spans `0..MAP_SIZE` on both axes.
pub const MAP_SIZE: i32 = 18432;

/// Cells per axis of the passability grid; matches the terrain layout.
pub const GRID_CELLS: usize = 288;

/// World units covered by one passability cell.
pub const GRID_CELL_SIZE: i32 = MAP_SIZE / GRID_CELLS as i32;

// Landmarks: the current Dota 2 map, every position shifted by half the map
// so Dota's origin sits at the center. The two sides are not mirror images;
// each carries its own table.

/// Center of the Radiant fountain area.
pub const RADIANT_FOUNTAIN_POS: Vec2 = Vec2::from_ints(1760, 2278);
/// Center of the Dire fountain area.
pub const DIRE_FOUNTAIN_POS: Vec2 = Vec2::from_ints(16624, 16064);
/// The Radiant Ancient.
pub const RADIANT_ANCIENT_POS: Vec2 = Vec2::from_ints(3296, 3864);
/// The Dire Ancient.
pub const DIRE_ANCIENT_POS: Vec2 = Vec2::from_ints(14744, 14216);
/// How far from the fountain center a hero appears, along both axes towards
/// the map center. Keeps the spawn clear of the fountain's collision radius.
pub const HERO_SPAWN_OFFSET: i32 = 280;

// Lanes. Lane 0 is mid across the middle; lane 1 is top, up the west edge
// and along the north edge; lane 2 is bottom along the south and east.

/// The mid lane.
pub const LANE_MID: u8 = 0;
/// Hero deaths that lose a one-lane demo match.
pub const DEMO_DEATH_LIMIT: u16 = 2;
/// The top lane.
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
/// click on that tree, and how far from one another two trees may stand.
pub const TREE_RADIUS: i32 = 48;
/// Imported trees this close to a lane centerline are dropped: the real
/// forest follows the real curved lanes, and this map walks straightened
/// ones.
pub const TREE_LANE_CLEAR: i32 = 450;
/// Tree-free radius around each fountain: the spawn pad. The rest of the
/// base keeps its real trees.
pub const TREE_BASE_CLEAR: i32 = 500;

/// Any unit inside this radius of a camp center blocks its spawn.
pub const CAMP_BOX_RADIUS: i32 = 300;
/// A neutral wakes when a hostile unit comes this close to it.
pub const NEUTRAL_AGGRO_RANGE: i32 = 240;
/// A neutral wakes when damaged or targeted from this far away.
pub const NEUTRAL_DAMAGE_AGGRO_RANGE: i32 = 1800;
/// How far from its spawn spot a neutral may stand before its aggro window
/// starts running down.
pub const NEUTRAL_GUARD_DISTANCE: i32 = 400;
/// Ticks a neutral stays awake beyond the guard distance.
pub const NEUTRAL_AGGRO_WINDOW: u32 = 5 * TICKS_PER_SECOND;
/// The shorter window a neutral gets when damage wakes it again soon after a
/// leash break.
pub const NEUTRAL_SHORT_WINDOW: u32 = 3 * TICKS_PER_SECOND;
/// Ticks after a leash break in which damage cannot wake a neutral.
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
/// Gold bounty one upgrade adds to a neutral.
pub const NEUTRAL_UPGRADE_GOLD: i32 = 1;
/// Experience one upgrade adds to a neutral.
pub const NEUTRAL_UPGRADE_XP: i32 = 5;
/// How far along its route a wave looks to know which way it faces.
pub const WAVE_FACING_LOOKAHEAD: i32 = 200;
/// World units between neighbours in a camp when it spawns.
pub const CAMP_SPAWN_SPACING: i32 = 64;
/// Tick of the first neutral spawn, one minute past the horn.
pub const FIRST_NEUTRAL_TICK: u32 = PREGAME_TICKS + 60 * TICKS_PER_SECOND;
/// Ticks between neutral spawn checks.
pub const NEUTRAL_SPAWN_PERIOD_TICKS: u32 = 60 * TICKS_PER_SECOND;
/// Neutral creep collision radius. Neutrals name no hull and take the
/// unit template's, which is the hero hull.
pub const NEUTRAL_RADIUS: i32 = 24;
/// Neutral creep fog light radius.
pub const NEUTRAL_VISION: i32 = 800;
/// How far a neutral creep looks for something to attack once awake.
pub const NEUTRAL_ACQUISITION: i32 = 500;

// Creep waves.

/// Ticks of pregame: the game clock counts up from minus this, thirty
/// seconds, and reaches zero when the first wave walks out.
pub const PREGAME_TICKS: u32 = 30 * TICKS_PER_SECOND;
/// Tick of the first creep wave.
pub const FIRST_WAVE_TICK: u32 = PREGAME_TICKS;
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
pub const RANGED_UPGRADE_GOLD: i32 = 6;
/// Experience one upgrade adds to a ranged creep.
pub const RANGED_UPGRADE_XP: i32 = 8;
/// World units between neighbours in a wave's rank.
pub const WAVE_SPAWN_SPACING: i32 = 48;
/// World units the ranged rank trails the front one.
pub const WAVE_SPAWN_RANK: i32 = 96;
/// Flagbearer magic resistance, percent.
pub const FLAGBEARER_MAGIC_RESIST_PCT: i32 = 40;

// What the three attributes are worth. Every one of them is read once, by the
// system that works out stats, and applies to whatever holds attributes at all.

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

// Attack speed. A unit swings at its own pace at [`BASE_ATTACK_SPEED`], and
// twice that pace at twice the number.

/// Attack speed with nothing added to it.
pub const BASE_ATTACK_SPEED: i32 = 100;
/// Slowest a unit may be brought to swing.
pub const MIN_ATTACK_SPEED: i32 = 20;
/// Fastest a unit may be brought to swing.
pub const MAX_ATTACK_SPEED: i32 = 700;

// Generic hero stats. Per-hero data replaces these when heroes arrive.

/// Hero health at level one, before strength.
pub const HERO_HP: i32 = 180;
/// Hero mana at level one, before intelligence.
pub const HERO_MANA: i32 = 84;
/// Hero movement speed, world units per second.
pub const HERO_MOVE_SPEED: i32 = 300;
/// Hero attack damage at level one, before the primary attribute.
pub const HERO_ATTACK_DAMAGE: i32 = 31;
/// Hero attack range.
pub const HERO_ATTACK_RANGE: i32 = 600;
/// Ticks between hero attack starts at [`BASE_ATTACK_SPEED`].
pub const HERO_ATTACK_INTERVAL: u32 = 63;
/// Ticks from attack start to the projectile leaving.
pub const HERO_ATTACK_POINT: u32 = 9;
/// Hero attack projectile speed, world units per second.
pub const HERO_PROJECTILE_SPEED: i32 = 900;
/// Hero armor, before agility.
pub const HERO_ARMOR: i32 = -1;
/// Hero magic resistance, percent.
pub const HERO_MAGIC_RESIST_PCT: i32 = 25;
/// Hero collision radius, the Dota hero hull.
pub const HERO_RADIUS: i32 = 24;
/// Hero fog light radius.
pub const HERO_VISION: i32 = 1800;
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
pub const HERO_MAX_LEVEL: u8 = 10;
/// Total experience required to sit at each level, indexed by `level - 1`.
pub const XP_THRESHOLDS: [i32; HERO_MAX_LEVEL as usize] =
    [0, 230, 600, 1080, 1660, 2260, 2980, 3730, 4620, 5550];
/// Ticks a dead hero waits before respawning, plus the per-level term.
pub const RESPAWN_BASE_TICKS: u32 = 120;
/// Additional respawn ticks per hero level.
pub const RESPAWN_PER_LEVEL_TICKS: u32 = 120;

// Creep stats: melee, ranged, siege.

/// Melee creep health.
pub const MELEE_CREEP_HP: i32 = 550;
/// Melee creep attack damage, the midpoint of 19 to 23.
pub const MELEE_CREEP_ATTACK_DAMAGE: i32 = 21;
/// Melee creep attack range.
pub const MELEE_CREEP_ATTACK_RANGE: i32 = 100;
/// How far a melee creep looks for something to attack.
pub const MELEE_CREEP_ACQUISITION: i32 = 500;
/// Melee creep armor.
pub const MELEE_CREEP_ARMOR: i32 = 2;
/// Melee creep collision radius, the Dota regular hull.
pub const MELEE_CREEP_RADIUS: i32 = 16;
/// Ticks from a melee creep's attack start to the hit; 0.467 s.
pub const MELEE_CREEP_ATTACK_POINT: u32 = 14;
/// Melee creep gold bounty, the midpoint of 34 to 39.
pub const MELEE_CREEP_BOUNTY: i32 = 36;
/// Melee creep experience.
pub const MELEE_CREEP_XP: i32 = 57;

/// Ranged creep health.
pub const RANGED_CREEP_HP: i32 = 300;
/// Ranged creep attack damage, the midpoint of 21 to 26.
pub const RANGED_CREEP_ATTACK_DAMAGE: i32 = 23;
/// Ranged creep attack range.
pub const RANGED_CREEP_ATTACK_RANGE: i32 = 500;
/// How far a ranged creep looks for something to attack.
pub const RANGED_CREEP_ACQUISITION: i32 = 600;
/// Ranged creep collision radius, the Dota small hull.
pub const RANGED_CREEP_RADIUS: i32 = 8;
/// Ticks from a ranged creep's attack start to the projectile leaving; 0.5 s.
pub const RANGED_CREEP_ATTACK_POINT: u32 = 15;
/// Ranged creep attack projectile speed, world units per second.
pub const RANGED_CREEP_PROJECTILE_SPEED: i32 = 900;
/// Ranged creep gold bounty, the midpoint of 43 to 52.
pub const RANGED_CREEP_BOUNTY: i32 = 47;
/// Ranged creep experience.
pub const RANGED_CREEP_XP: i32 = 69;

/// Siege creep health.
pub const SIEGE_CREEP_HP: i32 = 935;
/// Siege creep attack damage, the midpoint of 35 to 46.
pub const SIEGE_CREEP_ATTACK_DAMAGE: i32 = 40;
/// Siege creep attack range.
pub const SIEGE_CREEP_ATTACK_RANGE: i32 = 690;
/// How far a siege creep looks for something to attack.
pub const SIEGE_CREEP_ACQUISITION: i32 = 800;
/// Siege creep armor.
pub const SIEGE_CREEP_ARMOR: i32 = 0;
/// Siege creep magic resistance, percent.
pub const SIEGE_CREEP_MAGIC_RESIST_PCT: i32 = 80;
/// Siege creep collision radius, the Dota siege hull.
pub const SIEGE_CREEP_RADIUS: i32 = 16;
/// Ticks between siege creep attack starts; a base attack time of 3 s.
pub const SIEGE_CREEP_ATTACK_INTERVAL: u32 = 90;
/// Ticks from a siege creep's attack start to the projectile leaving; 0.7 s.
pub const SIEGE_CREEP_ATTACK_POINT: u32 = 21;
/// Siege creep attack projectile speed, world units per second.
pub const SIEGE_CREEP_PROJECTILE_SPEED: i32 = 1100;
/// Siege creep gold bounty, the midpoint of 59 to 72.
pub const SIEGE_CREEP_BOUNTY: i32 = 65;
/// Siege creep experience.
pub const SIEGE_CREEP_XP: i32 = 88;

/// Creep movement speed, world units per second.
pub const CREEP_MOVE_SPEED: i32 = 325;
/// Ticks between creep attack starts; a base attack time of 1 s.
pub const CREEP_ATTACK_INTERVAL: u32 = 30;
/// Creep fog light radius.
pub const CREEP_VISION: i32 = 750;

// Super and mega creeps: what a wave grows into once the enemy barracks that
// held it back have fallen.

/// Super melee creep health.
pub const SUPER_MELEE_HP: i32 = 700;
/// Super melee creep attack damage, the midpoint of 41 to 49.
pub const SUPER_MELEE_ATTACK_DAMAGE: i32 = 45;
/// Super melee creep armor.
pub const SUPER_MELEE_ARMOR: i32 = 3;
/// Super melee creep gold bounty, the midpoint of 20 to 26.
pub const SUPER_MELEE_BOUNTY: i32 = 23;
/// Super melee creep experience.
pub const SUPER_MELEE_XP: i32 = 25;
/// Super ranged creep health.
pub const SUPER_RANGED_HP: i32 = 475;
/// Super ranged creep attack damage, the midpoint of 46 to 51.
pub const SUPER_RANGED_ATTACK_DAMAGE: i32 = 48;
/// Super ranged creep armor.
pub const SUPER_RANGED_ARMOR: i32 = 1;
/// Super ranged creep gold bounty, the midpoint of 19 to 25.
pub const SUPER_RANGED_BOUNTY: i32 = 22;
/// Super ranged creep experience.
pub const SUPER_RANGED_XP: i32 = 22;
/// Super siege creep attack damage, the midpoint of 51 to 62.
pub const SUPER_SIEGE_ATTACK_DAMAGE: i32 = 56;
/// Ticks between mega melee creep attack starts: the same swing a tenth
/// faster.
pub const MEGA_MELEE_ATTACK_INTERVAL: u32 = 27;

// Buildings.

/// Tower health, indexed by tier minus one.
pub const TOWER_TIER_HP: [i32; 4] = [1800, 2000, 2200, 2600];
/// Tower attack damage, indexed by tier minus one.
pub const TOWER_TIER_DAMAGE: [i32; 4] = [110, 128, 144, 152];
/// Tower armor, indexed by tier minus one.
pub const TOWER_TIER_ARMOR: [i32; 4] = [12, 14, 15, 21];
/// Gold paid to the killer of a tower, indexed by tier minus one.
pub const TOWER_TIER_BOUNTY: [i32; 4] = [200, 250, 300, 350];
/// Tower attack range.
pub const TOWER_ATTACK_RANGE: i32 = 700;
/// Ticks between tower attack starts.
pub const TOWER_ATTACK_INTERVAL: u32 = 29;
/// Ticks from tower attack start to the projectile leaving.
pub const TOWER_ATTACK_POINT: u32 = 6;
/// Tower attack projectile speed, world units per second.
pub const TOWER_PROJECTILE_SPEED: i32 = 750;
/// Tower collision radius.
pub const TOWER_RADIUS: i32 = 40;
/// Tower fog light radius.
pub const TOWER_VISION: i32 = 1900;

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
/// Gold paid to the killer of a melee barracks.
pub const RAX_MELEE_BOUNTY: i32 = 225;
/// Gold paid to the killer of a ranged barracks.
pub const RAX_RANGED_BOUNTY: i32 = 150;
/// Barracks collision radius.
pub const RAX_RADIUS: i32 = 60;
/// Barracks fog light radius.
pub const RAX_VISION: i32 = 900;

/// Ancient health.
pub const ANCIENT_HP: i32 = 4500;
/// Ancient armor.
pub const ANCIENT_ARMOR: i32 = 13;
/// Ancient collision radius.
pub const ANCIENT_RADIUS: i32 = 72;
/// Ancient fog light radius.
pub const ANCIENT_VISION: i32 = 1200;

/// Fountain health. It is never lost: the fountain cannot be struck.
pub const FOUNTAIN_HP: i32 = 500;
/// Fountain attack damage, the midpoint of 290 to 310.
pub const FOUNTAIN_ATTACK_DAMAGE: i32 = 300;
/// Fountain attack range.
pub const FOUNTAIN_ATTACK_RANGE: i32 = 1200;
/// Ticks between fountain attack starts; 0.15 s, rounded up to the tick.
pub const FOUNTAIN_ATTACK_INTERVAL: u32 = 5;
/// Ticks from fountain attack start to the shot; the shortest wait a tick
/// clock has.
pub const FOUNTAIN_ATTACK_POINT: u32 = 1;
/// Fountain attack projectile speed, world units per second.
pub const FOUNTAIN_PROJECTILE_SPEED: i32 = 1400;
/// Fountain collision radius, the Dota tower hull.
pub const FOUNTAIN_RADIUS: i32 = 144;
/// Fountain fog light radius.
pub const FOUNTAIN_VISION: i32 = 1800;
/// Health restored per tick to allies inside the fountain area.
pub const FOUNTAIN_HEAL_HP_PER_TICK: i32 = 25;
/// Mana restored per tick to allies inside the fountain area.
pub const FOUNTAIN_HEAL_MANA_PER_TICK: i32 = 15;
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
/// Ticks a creep handed a target by an attack order keeps it before the
/// ordinary ranking may take it back, 2.33 seconds.
pub const ORDER_AGGRO_HOLD_TICKS: u32 = 70;
/// Game tick from which player units may aggro lane creeps unconditionally.
pub const FREE_AGGRO_TICK: u32 = FIRST_WAVE_TICK + 5 * 60 * TICKS_PER_SECOND;
/// How close to its own tier-one tower a lane creep may be aggroed before
/// [`FREE_AGGRO_TICK`].
pub const EARLY_AGGRO_TOWER_RANGE: i32 = 1500;
/// Ticks a lane creep chases a target that left its acquisition range,
/// 2.3 seconds.
pub const CREEP_CHASE_TICKS: u32 = 69;
/// How close a hero follows an ally it was ordered to attack but may not.
pub const FOLLOW_DISTANCE: i32 = 150;
/// Extra clearance added around structures when blocking grid cells.
pub const STEER_MARGIN: i32 = 8;
/// A path waypoint counts as reached within this distance.
pub const WAYPOINT_RADIUS: i32 = 40;
/// A stored path is recomputed once its goal drifted this far.
pub const REPATH_DRIFT: i32 = 128;
/// The smallest part of a step a slide keeps, as one over this. A walker
/// square against a body still works its way round, slowly.
pub const SLIDE_FLOOR_PART: i32 = 4;
/// Ticks a creep stands unable to move before it shoves through bodies.
pub const MARCH_SHOVE_TICKS: u32 = 30;

/// How far a body may be eased out of another one in a tick, in units.
pub const SEPARATION_STEP: i32 = 4;

pub const TRACE_CLEARANCE: i32 = 24;
/// How many times a blocked step is halved looking for one that fits.
pub const STEP_FIT_TRIES: u32 = 3;
/// Ticks a hero recovers after a swing. Cancelled by any order.
pub const HERO_ATTACK_BACKSWING: u32 = 12;
/// Ticks a creep recovers after a swing.
pub const CREEP_ATTACK_BACKSWING: u32 = 15;
/// Ticks a tower recovers after a shot.
pub const TOWER_ATTACK_BACKSWING: u32 = 4;
/// Ticks the fountain recovers after a shot.
pub const FOUNTAIN_ATTACK_BACKSWING: u32 = 2;
/// How fast a unit turns, in brads per tick.
///
/// The shipped `MovementTurnRate` is radians per 0.03 seconds; a half, which
/// every lane creep carries, is this many brads over a tick of a thirtieth.
pub const TURN_RATE_BRADS: u16 = 5795;
/// A unit swings only when facing within this error of its target, in brads.
///
/// An eighth of a right angle less a touch: the 11.5 degrees Dota allows an
/// order before the unit has to come round first.
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
pub const ULT_LEVEL_FLOORS: [u8; 3] = [6, 8, 10];

// Sylla: crit passive / attack speed buff / bouncing projectile / multishot.

/// Chance for a ranged attack to miss a target on higher ground.
pub const UPHILL_MISS: Ratio = Ratio::new(1, 4);

/// Fog blocker nodes further apart than this belong to different walls of
/// the same named group, not to one span.
pub const FOW_BLOCKER_SPAN: i32 = 600;

/// Chance of a critical strike per crit level.
pub const SYLLA_CRIT_CHANCE: [Ratio; 4] = [
    Ratio::new(1, 5),
    Ratio::new(1, 4),
    Ratio::new(3, 10),
    Ratio::new(7, 20),
];
/// Critical strike damage per crit level, percent of a normal hit.
pub const SYLLA_CRIT_MULT_PCT: [i32; 4] = [175, 200, 225, 250];
/// Frenzy mana cost per level.
pub const SYLLA_FRENZY_MANA: [i32; 4] = [30, 40, 50, 60];
/// Frenzy cooldown per level, ticks.
pub const SYLLA_FRENZY_COOLDOWN: [u32; 4] = [450, 420, 390, 360];
/// Attack speed Frenzy adds per level.
pub const SYLLA_FRENZY_ATTACK_SPEED: [i32; 4] = [25, 39, 56, 79];
/// Frenzy duration, ticks.
pub const SYLLA_FRENZY_TICKS: u32 = 180;
/// Bounce mana cost per level.
pub const SYLLA_BOUNCE_MANA: [i32; 4] = [90, 100, 110, 120];
/// Bounce cooldown per level, ticks.
pub const SYLLA_BOUNCE_COOLDOWN: [u32; 4] = [300, 270, 240, 210];
/// Bounce magical damage per hit per level.
pub const SYLLA_BOUNCE_DAMAGE: [i32; 4] = [70, 140, 210, 280];
/// Extra targets after the first per level.
pub const SYLLA_BOUNCE_COUNT: [u8; 4] = [2, 4, 6, 8];
/// Cast range of the bounce.
pub const SYLLA_BOUNCE_CAST_RANGE: i32 = 550;
/// How far the bounce jumps between targets.
pub const SYLLA_BOUNCE_RANGE: i32 = 500;
/// Bounce projectile speed, world units per second.
pub const SYLLA_BOUNCE_SPEED: i32 = 900;
/// Multishot mana cost per level.
pub const SYLLA_MULTI_MANA: [i32; 3] = [100, 150, 200];
/// Multishot cooldown per level, ticks.
pub const SYLLA_MULTI_COOLDOWN: [u32; 3] = [2100, 1800, 1500];
/// Multishot damage per level, percent of attack damage.
pub const SYLLA_MULTI_DMG_PCT: [i32; 3] = [80, 100, 120];
/// Radius the multishot volley covers.
pub const SYLLA_MULTI_RADIUS: i32 = 700;

// Items and the shop.

/// The flat bonuses and price of one purchasable item.
///
/// `charges` above zero makes the item a consumable with that many uses and
/// no bonuses while carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemDef {
    /// Price in gold.
    pub cost: i32,
    /// Movement speed added.
    pub move_speed: i32,
    /// Attack damage added.
    pub damage: i32,
    /// Armor added.
    pub armor: i32,
    /// Maximum health added.
    pub hp: i32,
    /// Maximum mana added.
    pub mana: i32,
    /// Uses a consumable carries. Zero for carried bonuses.
    pub charges: u8,
}

const fn passive(
    cost: i32,
    move_speed: i32,
    damage: i32,
    armor: i32,
    hp: i32,
    mana: i32,
) -> ItemDef {
    ItemDef {
        cost,
        move_speed,
        damage,
        armor,
        hp,
        mana,
        charges: 0,
    }
}

/// The catalog, indexed by `ItemId`: Boots of Speed, Blades of Attack,
/// Broadsword, Claymore, Platemail, Vitality Booster, Energy Booster,
/// Healing Salve, Clarity.
pub const ITEMS: [ItemDef; 9] = [
    passive(500, 45, 0, 0, 0, 0),
    passive(450, 0, 9, 0, 0, 0),
    passive(1000, 0, 16, 0, 0, 0),
    passive(1400, 0, 20, 0, 0, 0),
    passive(1400, 0, 0, 10, 0, 0),
    passive(1100, 0, 0, 0, 250, 0),
    passive(800, 0, 0, 0, 0, 250),
    ItemDef {
        cost: 110,
        move_speed: 0,
        damage: 0,
        armor: 0,
        hp: 0,
        mana: 0,
        charges: 1,
    },
    ItemDef {
        cost: 95,
        move_speed: 0,
        damage: 0,
        armor: 0,
        hp: 0,
        mana: 0,
        charges: 1,
    },
];
/// Ticks a felled tree takes to come back, five minutes.
pub const TREE_REGROW_TICKS: u32 = 5 * 60 * TICKS_PER_SECOND;
/// Ticks a planted tree stands before it goes on its own, forty seconds.
pub const PLANTED_TREE_TICKS: u32 = 40 * TICKS_PER_SECOND;
/// Ticks a hero waits between scrolls, whichever scroll it holds.
pub const SCROLL_WAIT_TICKS: u32 = 2100;

// The courier.

/// Health a courier stands up with.
pub const COURIER_HP: i32 = 250;
/// World units a second a courier flies.
pub const COURIER_MOVE_SPEED: i32 = 380;
/// How far a courier sees.
pub const COURIER_VISION: i32 = 500;
/// Ticks a courier waits before it comes back.
pub const COURIER_RESPAWN_TICKS: u32 = 25 * TICKS_PER_SECOND;
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
/// How far from the shop a courier stands to reach the stash.
pub const COURIER_STASH_RANGE: i32 = 700;
/// How close a courier must be to hand over what it carries.
pub const COURIER_DELIVER_RANGE: i32 = 200;

// Pudge.

/// Mana the hook costs, by level.
pub const HOOK_MANA: [i32; 4] = [110, 120, 130, 140];
/// Ticks between hooks, by level.
pub const HOOK_COOLDOWN: [u32; 4] = [810, 690, 570, 450];
/// How far the hook flies, in world units.
pub const HOOK_RANGE: i32 = 1100;
/// How fast the hook flies, in world units a second.
pub const HOOK_SPEED: i32 = 1600;
/// How wide the hook catches, in world units.
pub const HOOK_RADIUS: i32 = 100;
/// Damage the hook deals, by level.
pub const HOOK_DAMAGE: [i32; 4] = [90, 180, 270, 360];
/// How far the rot reaches, in world units.
pub const ROT_RADIUS: i32 = 250;
/// Damage the rot deals a second, by level.
pub const ROT_DAMAGE_PER_SECOND: [i32; 4] = [30, 60, 90, 120];
/// Percent of speed the rot takes, by level.
pub const ROT_SLOW_PCT: [i32; 4] = [10, 15, 20, 25];
/// How near a death has to be to feed the flesh heap, in world units.
pub const FLESH_HEAP_RANGE: i32 = 700;
/// Health one stack of the flesh heap is worth.
pub const FLESH_HEAP_HP: i32 = 14;
/// Magic resistance each level of the flesh heap gives, percent.
pub const FLESH_HEAP_RESIST_PCT: [i32; 4] = [4, 8, 12, 16];
/// Mana the dismember costs, by level.
pub const DISMEMBER_MANA: [i32; 3] = [100, 130, 170];
/// Ticks between dismembers, by level.
pub const DISMEMBER_COOLDOWN: [u32; 3] = [900, 750, 600];
/// How far the dismember reaches, in world units.
pub const DISMEMBER_RANGE: i32 = 150;
/// Ticks a dismember holds what it caught.
pub const DISMEMBER_TICKS: u32 = 90;
/// Damage the dismember deals a second, by level.
pub const DISMEMBER_DAMAGE_PER_SECOND: [i32; 3] = [75, 125, 175];

// Shadow Fiend.

/// How far in front of the caster a raze lands, in world units, from the
/// nearest reach to the farthest.
pub const RAZE_DISTANCE: [i32; 3] = [200, 450, 700];
/// How wide a raze burns, in world units.
pub const RAZE_RADIUS: i32 = 250;
/// Damage a raze deals, by level.
pub const RAZE_DAMAGE: [i32; 4] = [90, 160, 230, 300];
/// Mana a raze costs, by level.
pub const RAZE_MANA: [i32; 4] = [75, 80, 85, 90];
/// Ticks between two razes of the same reach, by level.
pub const RAZE_COOLDOWN: [u32; 4] = [10 * TICKS_PER_SECOND; 4];
/// Souls one unit brought down is worth to whoever gathers them.
pub const SOULS_PER_UNIT: u32 = 1;
/// Souls one hero brought down is worth to whoever gathers them.
pub const SOULS_PER_HERO: u32 = 3;
/// Souls that may be held, by necromastery level.
pub const NECRO_SOUL_CAP: [u32; 4] = [12, 16, 20, 24];
/// Attack damage one soul is worth.
pub const DAMAGE_PER_SOUL: i32 = 2;
/// How far the presence reaches, in world units.
pub const PRESENCE_RADIUS: i32 = 900;
/// Armor the presence takes from an enemy standing in it, by level.
pub const PRESENCE_ARMOR: [i32; 4] = [2, 3, 4, 5];
/// Ticks the presence lingers on an enemy that walks out of it.
pub const PRESENCE_LINGER_TICKS: u32 = TICKS_PER_SECOND / 2;
/// How far the requiem reaches, in world units.
pub const REQUIEM_RADIUS: i32 = 900;
/// Damage the requiem deals for each soul held, by level.
pub const REQUIEM_DAMAGE_PER_SOUL: [i32; 3] = [8, 11, 14];
/// Percent of speed the requiem takes from what it catches, by level.
pub const REQUIEM_SLOW_PCT: [i32; 3] = [30, 40, 50];
/// Ticks the requiem's slow holds.
pub const REQUIEM_SLOW_TICKS: u32 = 2 * TICKS_PER_SECOND;
/// Mana the requiem costs, by level.
pub const REQUIEM_MANA: [i32; 3] = [150, 175, 200];
/// Ticks between requiems, by level.
pub const REQUIEM_COOLDOWN: [u32; 3] = [
    120 * TICKS_PER_SECOND,
    110 * TICKS_PER_SECOND,
    100 * TICKS_PER_SECOND,
];

/// Ticks between the beats of anything that burns over time.
pub const BURN_PERIOD_TICKS: u32 = 3;
/// Inventory slots, where items work.
pub const INVENTORY_SLOTS: usize = 6;
/// Backpack slots, where items are carried inert.
pub const BACKPACK_SLOTS: usize = 3;
/// Stash slots at the home shop.
pub const STASH_SLOTS: usize = 6;
/// Ticks an item stays muted after leaving the backpack for the inventory.
pub const BACKPACK_MUTE_TICKS: u32 = 180;
/// How close to the home fountain the shop, the stash and selling work.
pub const SHOP_RANGE: i32 = 1000;
/// Percent of the price a sale returns.
pub const SELL_PCT: i32 = 50;
/// Ticks after purchase in which an unused item refunds in full.
pub const SELL_REFUND_TICKS: u32 = 300;
/// How far an item may be laid on the ground or handed to another bag, in
/// world units.
pub const PUT_ITEM_RANGE: i32 = 400;
/// How close a unit must be to take an item off the ground, in world units.
pub const TAKE_ITEM_RANGE: i32 = 150;
/// How far from an enemy cast an item gains a charge from it.
pub const MAGIC_CHARGE_RANGE: i32 = 1200;
/// How far back along its line a blink steps looking for open ground.
pub const BLINK_STEP_BACK: i32 = 64;
/// Steps back a blink takes before it gives up on finding open ground.
pub const BLINK_STEP_TRIES: u32 = 20;
/// Denominator scale of the armor formula: each point of armor adds
/// `ARMOR_SCALE` to a base of one hundred.
pub const ARMOR_SCALE: i32 = 6;

// Economy.

/// Gold each seat starts the match with.
pub const STARTING_GOLD: i32 = 600;
/// One gold arrives every this many ticks.
pub const PASSIVE_GOLD_PERIOD_TICKS: u32 = 30;
/// Gold a dying hero loses, per level of it, never more than it holds.
pub const DEATH_GOLD_LOSS_PER_LEVEL: i32 = 30;
/// Gold for killing a hero, before the streak bonus.
pub const HERO_KILL_BOUNTY_BASE: i32 = 200;
/// Extra gold per kill in the victim's streak.
pub const HERO_KILL_BOUNTY_PER_STREAK: i32 = 50;
/// The streak bonus stops growing past this many kills.
pub const HERO_KILL_STREAK_CAP: i32 = 6;
/// Experience for killing a hero, before the per-level term.
pub const HERO_KILL_XP_BASE: i32 = 100;
/// Extra experience per level of the killed hero.
pub const HERO_KILL_XP_PER_LEVEL: i32 = 40;
/// Radius around a death within which enemy heroes receive experience.
pub const XP_RADIUS: i32 = 1500;

/// A friendly creep may be denied when its health is strictly below this
/// fraction of the maximum, expressed as a percent.
pub const DENY_HP_PCT: i32 = 50;

/// A friendly building may be denied when its health is strictly below this
/// fraction of the maximum, expressed as a percent.
pub const DENY_BUILDING_HP_PCT: i32 = 10;
/// Denied creeps grant this percent of their experience.
pub const DENIED_XP_PCT: i32 = 50;

/// Helper for constants that are distances: a whole number of world units.
pub const fn units(n: i32) -> Fixed {
    Fixed::from_int(n)
}
