//! Where things stand on a map, and the lanes creeps walk down it.
//!
//! Everything here follows from the map alone, so it is the same for every
//! match played on it.

use bota_proto::{Team, Vec2};

use crate::game::{Clearance, Obstacles, Planner, rules};

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
    // The lane polylines are the map's, not each tree's: laying them once
    // keeps a map's worth of trees from laying them again for every tree.
    let lanes: Vec<(u8, Vec<Vec2>)> = if lane_clear > 0 {
        map.lanes()
            .map(|lane| (lane, lane_polyline(map, lane)))
            .collect()
    } else {
        Vec::new()
    };
    map.trees
        .iter()
        .map(|&(x, y)| Vec2::from_ints(i32::from(x), i32::from(y)))
        .filter(|&pos| {
            for (_, line) in &lanes {
                let offset = line
                    .windows(2)
                    .map(|s| crate::game::segment_distance_squared(pos, s[0], s[1]))
                    .min()
                    .expect("a lane has at least one segment");
                if offset < lane_clear {
                    return false;
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

/// The landmarks a team's creeps march through on a lane, spawner first.
fn lane_landmarks(map: &crate::game::MapDef, team: Team, lane: u8) -> Vec<Vec2> {
    let mut line = vec![creep_spawn_pos(map, team, lane)];
    line.extend(lane_route(map, team, lane));
    line
}

/// The walked route of every lane, both sides, indexed by team then lane,
/// with everything the map starts with standing.
pub fn lane_routes(map: &'static crate::game::MapDef) -> [[Vec<Vec2>; 3]; 2] {
    let field = Clearance::of_map(map);
    let mut planner = Planner::new();
    lane_routes_on(map, &field, &mut planner)
}

/// The walked route of every lane, both sides, indexed by team then lane,
/// on the ground as a field has it.
pub fn lane_routes_on(
    map: &crate::game::MapDef,
    field: &Clearance,
    planner: &mut Planner,
) -> [[Vec<Vec2>; 3]; 2] {
    let ob = Obstacles { field, extra: &[] };
    let mut build = |team: Team| {
        [
            walk_lane(map, &ob, planner, team, rules::LANE_MID),
            walk_lane(map, &ob, planner, team, rules::LANE_TOP),
            walk_lane(map, &ob, planner, team, rules::LANE_BOT),
        ]
    };
    let radiant = build(Team::Radiant);
    let dire = build(Team::Dire);
    [radiant, dire]
}

/// One lane's walked route: a stop beside each landmark, with a found path
/// laid between each pair so the march goes around what stands in the way.
///
/// Landmarks are tower positions, and a tower closes the ground it stands
/// on: the stop is beside it on its lane side, away from the base it
/// guards. The march walks past its own towers on the way out of its base
/// and comes up to the enemy's from the lane.
fn walk_lane(
    map: &crate::game::MapDef,
    ob: &Obstacles,
    planner: &mut Planner,
    team: Team,
    lane: u8,
) -> Vec<Vec2> {
    let marks = lane_landmarks(map, team, lane);
    if marks.len() < 2 {
        return Vec::new();
    }
    let room = rules::units(rules::WIDEST_MARCHER);
    let stops: Vec<Vec2> = marks
        .iter()
        .enumerate()
        .map(|(at, &mark)| {
            let own = guarded_by(map, mark) == Some(team);
            let toward = if own && at + 1 < marks.len() {
                marks[at + 1]
            } else {
                marks[at.saturating_sub(1)]
            };
            crate::game::open_beside(ob, mark, toward, room)
        })
        .collect();
    let mut out = Vec::new();
    for leg in stops.windows(2) {
        out.extend(planner.find_path(ob, leg[0], leg[1], room));
        if out.last() != Some(&leg[1]) {
            out.push(leg[1]);
        }
    }
    out
}

/// The team whose tower or Ancient stands at a spot, if one does.
fn guarded_by(map: &crate::game::MapDef, at: Vec2) -> Option<Team> {
    let stands = |towers: &[(u8, u8, Vec2)]| towers.iter().any(|&(_, _, pos)| pos == at);
    if stands(map.radiant_towers) || map.ancients[0] == Some(at) {
        Some(Team::Radiant)
    } else if stands(map.dire_towers) || map.ancients[1] == Some(at) {
        Some(Team::Dire)
    } else {
        None
    }
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

/// Which waypoint of a lane route a marcher aims at next, never one it has
/// already passed.
///
/// A waypoint is passed once the marcher stands within
/// [`rules::LANE_WAYPOINT_RADIUS`] of it, or beside or beyond it along the
/// leg to the next with its body able to walk straight to that next one
/// from where it stands. Several may be passed at once.
pub fn advance_waypoint(
    field: &Clearance,
    route: &[Vec2],
    from: usize,
    at: Vec2,
    collision: bota_proto::Fixed,
) -> usize {
    let radius = rules::units(rules::LANE_WAYPOINT_RADIUS);
    let room = crate::game::plan_radius(collision);
    let mut step = from.min(route.len().saturating_sub(1));
    while step + 1 < route.len() {
        let (here, next) = (route[step], route[step + 1]);
        let beyond = {
            let ax = i64::from(at.x.raw) - i64::from(here.x.raw);
            let ay = i64::from(at.y.raw) - i64::from(here.y.raw);
            let lx = i64::from(next.x.raw) - i64::from(here.x.raw);
            let ly = i64::from(next.y.raw) - i64::from(here.y.raw);
            ax * lx + ay * ly > 0
        };
        let passed = at.within(here, radius) || (beyond && field.capsule_clear(at, next, room));
        if !passed {
            break;
        }
        step += 1;
    }
    step
}
