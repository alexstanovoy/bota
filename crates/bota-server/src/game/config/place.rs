//! Where things stand on a map, and the lanes creeps walk down it.
//!
//! Everything here follows from the map alone, so it is the same for every
//! match played on it.

use bota_proto::{Team, Vec2};

use crate::game::{Ground, PassGrid, rules};

/// The fountain position of a team. The jungle's is the map center: it has
/// no fountain, and nothing ever stands there.
pub fn fountain_pos(map: &crate::game::MapDef, team: Team) -> Vec2 {
    match team {
        Team::Radiant => map.fountains[0],
        Team::Dire => map.fountains[1],
        Team::Neutral => Vec2::from_ints(rules::MAP_SIZE / 2, rules::MAP_SIZE / 2),
    }
}

/// Where a team's hero appears, beside the fountain rather than inside it.
pub fn hero_spawn_pos(map: &crate::game::MapDef, team: Team) -> Vec2 {
    let offset = match team {
        Team::Radiant => Vec2::from_ints(rules::HERO_SPAWN_OFFSET, rules::HERO_SPAWN_OFFSET),
        Team::Dire => Vec2::from_ints(-rules::HERO_SPAWN_OFFSET, -rules::HERO_SPAWN_OFFSET),
        Team::Neutral => Vec2::ZERO,
    };
    fountain_pos(map, team) + offset
}

/// The mirror of a position through the map center.
pub fn mirror(pos: Vec2) -> Vec2 {
    Vec2::from_ints(rules::MAP_SIZE, rules::MAP_SIZE) - pos
}

/// Every tree on the map: its own forest, with the lane corridors the map
/// asks for and both spawn pads kept clear.
pub fn tree_positions(map: &crate::game::MapDef) -> Vec<Vec2> {
    let lane_clear = {
        let r = i64::from(rules::units(map.lane_clear).raw);
        r * r
    };
    let base_clear = rules::units(rules::TREE_BASE_CLEAR);
    map.trees
        .iter()
        .map(|&(x, y)| Vec2::from_ints(i32::from(x), i32::from(y)))
        .filter(|&pos| {
            if lane_clear > 0 {
                for lane in map.lanes() {
                    if lane_offset_squared(map, lane, pos) < lane_clear {
                        return false;
                    }
                }
            }
            !pos.within(map.fountains[0], base_clear) && !pos.within(map.fountains[1], base_clear)
        })
        .collect()
}

/// The physical centerline of a lane, Radiant base first.
///
/// On a map that says so, the line runs through every tower of the lane, so
/// a wave walks from tower to tower and cannot wander past one out of its
/// own acquisition range; on one whose corners trace the real road, the
/// towers stand beside the line rather than on it. A side with no Ancient
/// anchors its end at its own wave spawner instead, so a winning wave still
/// marches into the enemy base.
pub fn lane_polyline(map: &crate::game::MapDef, lane: u8) -> Vec<Vec2> {
    let tower_of = |table: &[(u8, u8, Vec2)], tier: u8| {
        table
            .iter()
            .find(|&&(tl, tt, _)| tl == lane && tt == tier)
            .map(|&(_, _, pos)| pos)
    };
    let anchor =
        |side: usize| map.ancients[side].unwrap_or(map.creep_spawns[side][usize::from(lane)]);
    let mut line = vec![anchor(0)];
    if map.lane_through_towers {
        for tier in [3u8, 2, 1] {
            if let Some(pos) = tower_of(map.radiant_towers, tier) {
                line.push(pos);
            }
        }
    }
    if let Some(corners) = map.lane_corners.get(usize::from(lane)) {
        line.extend_from_slice(corners);
    }
    if map.lane_through_towers {
        for tier in [1u8, 2, 3] {
            if let Some(pos) = tower_of(map.dire_towers, tier) {
                line.push(pos);
            }
        }
    }
    line.push(anchor(1));
    line
}

/// The waypoints a team's creeps push through on a lane, enemy base last.
///
/// The wave begins at its spawner, which stands somewhere along the lane —
/// on three lanes ahead of its own rearmost tower — so the route takes only
/// what lies past the spawner's own place on the line, and a fresh wave
/// never walks back towards its base first.
pub fn lane_route(map: &crate::game::MapDef, team: Team, lane: u8) -> Vec<Vec2> {
    let mut line = lane_polyline(map, lane);
    if team == Team::Dire {
        line.reverse();
    }
    let spawn = creep_spawn_pos(map, team, lane);
    let mut nearest = (0usize, i64::MAX);
    for (at, seg) in line.windows(2).enumerate() {
        let d = crate::game::segment_distance_squared(spawn, seg[0], seg[1]);
        if d < nearest.1 {
            nearest = (at, d);
        }
    }
    line.split_off(nearest.0 + 1)
}

/// The passability grid of a map: its terrain, its buildings and its forest.
///
/// Built from the map alone, so the routes found on it never depend on which
/// world asked first.
pub fn build_grid(map: &crate::game::MapDef) -> PassGrid {
    let ground = Ground::of(map);
    let mut grid = PassGrid::open();
    for cy in 0..rules::GRID_CELLS {
        for cx in 0..rules::GRID_CELLS {
            if !ground.cell_walkable(cx, cy) {
                grid.close_cell(cx, cy);
            }
        }
    }
    let mut block = |pos: Vec2, radius: bota_proto::Fixed| {
        grid.block_circle(pos, crate::game::structure_clearance(radius));
    };
    for at in map.fountains {
        block(at, rules::units(rules::FOUNTAIN_RADIUS));
    }
    for at in map.ancients.into_iter().flatten() {
        block(at, rules::units(rules::ANCIENT_RADIUS));
    }
    for &(_, _, at) in map.radiant_towers.iter().chain(map.dire_towers) {
        block(at, rules::units(rules::TOWER_RADIUS));
    }
    for &(_, _, at) in map.barracks[0].iter().chain(map.barracks[1]) {
        block(at, rules::units(rules::RAX_RADIUS));
    }
    for at in tree_positions(map) {
        block(at, rules::units(rules::TREE_RADIUS));
    }
    grid
}

/// The landmarks a team's creeps march through on a lane, spawner first.
fn lane_landmarks(map: &crate::game::MapDef, team: Team, lane: u8) -> Vec<Vec2> {
    let mut line = vec![creep_spawn_pos(map, team, lane)];
    line.extend(lane_route(map, team, lane));
    line
}

/// The walked route of every lane, both sides, indexed by team then lane.
///
/// Every match runs the same map, so the routes are found once and shared.
pub fn lane_routes(map: &'static crate::game::MapDef) -> &'static [[Vec<Vec2>; 3]; 2] {
    static ROUTES: std::sync::OnceLock<Vec<[[Vec<Vec2>; 3]; 2]>> = std::sync::OnceLock::new();
    &ROUTES.get_or_init(|| {
        crate::game::MAPS
            .iter()
            .map(|m| {
                let grid = build_grid(m);
                let build = |team: Team| {
                    [
                        walk_lane(m, &grid, team, rules::LANE_MID),
                        walk_lane(m, &grid, team, rules::LANE_TOP),
                        walk_lane(m, &grid, team, rules::LANE_BOT),
                    ]
                };
                [build(Team::Radiant), build(Team::Dire)]
            })
            .collect()
    })[map.index()]
}

/// One lane's walked route: the landmarks, with a found path laid between
/// each pair so the march goes around what stands in the way.
fn walk_lane(map: &crate::game::MapDef, grid: &PassGrid, team: Team, lane: u8) -> Vec<Vec2> {
    let marks = lane_landmarks(map, team, lane);
    let mut out = Vec::new();
    for leg in marks.windows(2) {
        out.extend(crate::game::find_path(grid, leg[0], leg[1]));
        // Landmarks are tower positions, and a tower closes the ground it
        // stands on: the march aims beside it, not at it.
        out.push(crate::game::nearest_open(grid, leg[1]));
    }
    out
}

/// Squared distance from a lane's centerline.
pub fn lane_offset_squared(map: &crate::game::MapDef, lane: u8, pos: Vec2) -> i64 {
    let line = lane_polyline(map, lane);
    line.windows(2)
        .map(|s| crate::game::segment_distance_squared(pos, s[0], s[1]))
        .min()
        .expect("a lane has at least one segment")
}

/// The creep spawn position of a team on a lane. The jungle runs no lanes.
pub fn creep_spawn_pos(map: &crate::game::MapDef, team: Team, lane: u8) -> Vec2 {
    match team {
        Team::Radiant => map.creep_spawns[0][usize::from(lane)],
        Team::Dire => map.creep_spawns[1][usize::from(lane)],
        Team::Neutral => Vec2::from_ints(rules::MAP_SIZE / 2, rules::MAP_SIZE / 2),
    }
}

/// Where a team sits in the per-team route tables. The jungle marches
/// nowhere and answers zero.
pub fn team_index(team: Team) -> usize {
    match team {
        Team::Radiant | Team::Neutral => 0,
        Team::Dire => 1,
    }
}

/// Which waypoint of a route a walker aims at next.
///
/// A creep aims at its next waypoint and nothing else: it is never pulled
/// sideways towards the centreline, and a waypoint counts as reached from
/// anywhere inside [`rules::LANE_WAYPOINT_RADIUS`] — but only while the
/// ground to the waypoint after it is clear. The radius spans a tower, and a
/// waypoint that exists to route around one must not be cleared from its far
/// side. Several waypoints may fall inside the radius at once, and all of
/// them are cleared together.
pub fn advance_waypoint(grid: &PassGrid, route: &[Vec2], from: usize, at: Vec2) -> usize {
    let radius = rules::units(rules::LANE_WAYPOINT_RADIUS);
    let mut step = from.min(route.len().saturating_sub(1));
    while step + 1 < route.len()
        && at.within(route[step], radius)
        && crate::game::grid_los(grid, at, route[step + 1])
    {
        step += 1;
    }
    step
}
