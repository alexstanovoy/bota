//! Where things stand on a map, and the lanes creeps walk down it.
//!
//! Everything here follows from the map alone, so it is the same for every
//! match played on it.

use bota_core::{creep_spawn_pos, lane_polyline, lane_route, rules};
use bota_proto::{Team, Vec2};

use crate::game::{Clearance, MapTerrain, Obstacles, Planner};

/// Every tree on the map: its own forest, with the lane corridors the map
/// asks for and both spawn pads kept clear.
pub fn tree_positions(map: &bota_core::MapDef, terrain: &MapTerrain) -> Vec<Vec2> {
    let lane_clear = {
        let r = i64::from(rules::units(terrain.lane_clear).raw);
        r * r
    };
    let base_clear = rules::units(rules::TREE_BASE_CLEAR);
    let lanes: Vec<Vec<Vec2>> = if lane_clear > 0 {
        map.lanes().map(|lane| lane_polyline(map, lane)).collect()
    } else {
        Vec::new()
    };
    let off_lanes = |pos: Vec2| {
        lanes.iter().all(|line| {
            line.windows(2)
                .map(|s| bota_core::segment_distance_squared(pos, s[0], s[1]))
                .min()
                .expect("a lane has at least one segment")
                >= lane_clear
        })
    };
    terrain
        .trees
        .iter()
        .map(|&(x, y)| Vec2::from_ints(i32::from(x), i32::from(y)))
        .filter(|&pos| {
            off_lanes(pos)
                && !pos.within(map.fountains[0], base_clear)
                && !pos.within(map.fountains[1], base_clear)
        })
        .collect()
}

/// The walked route of every lane, both sides, indexed by team then lane,
/// with everything the map starts with standing.
pub fn lane_routes(map: &'static bota_core::MapDef) -> [[Vec<Vec2>; 3]; 2] {
    let field = Clearance::of_map(map);
    let mut planner = Planner::new();
    lane_routes_on(map, &field, &mut planner)
}

/// The walked route of every lane, both sides, indexed by team then lane,
/// on the ground as a field has it.
pub fn lane_routes_on(
    map: &bota_core::MapDef,
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
    [build(Team::Radiant), build(Team::Dire)]
}

/// One lane's walked route: a stop beside each landmark, with a found path
/// laid between each pair.
///
/// A stop is the open spot beside its landmark towards the next landmark
/// for the team's own tower or Ancient, towards the previous one otherwise.
fn walk_lane(
    map: &bota_core::MapDef,
    ob: &Obstacles,
    planner: &mut Planner,
    team: Team,
    lane: u8,
) -> Vec<Vec2> {
    // The landmarks the team's creeps march through, spawner first.
    let mut marks = vec![creep_spawn_pos(map, team, lane)];
    marks.extend(lane_route(map, team, lane));
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
fn guarded_by(map: &bota_core::MapDef, at: Vec2) -> Option<Team> {
    let stands = |towers: &[(u8, u8, Vec2)]| towers.iter().any(|&(_, _, pos)| pos == at);
    if stands(map.radiant_towers) || map.ancients[0] == Some(at) {
        Some(Team::Radiant)
    } else if stands(map.dire_towers) || map.ancients[1] == Some(at) {
        Some(Team::Dire)
    } else {
        None
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
