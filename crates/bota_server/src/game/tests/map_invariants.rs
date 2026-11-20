//! What any map generated from an install must hold: the shape of the game, not its numbers. A new
//! patch's map that breaks one of these was read wrongly, whatever its numbers are.

use std::collections::VecDeque;

use bota_core::{MAP2_ID, MapDef, RuneSpotKind, hero_spawn_pos, map_of, rules};
use bota_proto::{MapId, Team, Vec2};

use crate::game::{Clearance, terrain_of, tree_positions};

const CELLS: usize = rules::GRID_CELLS;

/// The full map and the solo-mid map, which plays on its ground.
fn dota_maps() -> [&'static MapDef; 2] {
    [map_of(MapId(0)), map_of(MAP2_ID)]
}

fn cell(at: Vec2) -> (usize, usize) {
    let index = |v: i32| (v / rules::GRID_CELL_SIZE) as usize;
    (index(at.x.to_int()), index(at.y.to_int()))
}

fn distance_squared(a: Vec2, b: Vec2) -> i64 {
    let (dx, dy) = (
        i64::from(a.x.to_int() - b.x.to_int()),
        i64::from(a.y.to_int() - b.y.to_int()),
    );
    dx * dx + dy * dy
}

/// The cells a walker may cross: open ground the trees left open, each tree closing the four
/// cells about the corner it stands on.
fn open_cells(map: &'static MapDef) -> Vec<bool> {
    let field = Clearance::of_map(map);
    let mut open: Vec<bool> = (0..CELLS * CELLS)
        .map(|i| field.terrain().cell_open(i % CELLS, i / CELLS))
        .collect();
    for tree in tree_positions(map, terrain_of(map)) {
        let (tx, ty) = (tree.x.to_int() / 64, tree.y.to_int() / 64);
        for (cx, cy) in [(tx - 1, ty - 1), (tx, ty - 1), (tx - 1, ty), (tx, ty)] {
            if (0..CELLS as i32).contains(&cx) && (0..CELLS as i32).contains(&cy) {
                open[cy as usize * CELLS + cx as usize] = false;
            }
        }
    }
    open
}

/// The cells reachable from `from` through open cells, side by side.
fn reachable(open: &[bool], from: (usize, usize)) -> Vec<bool> {
    let mut seen = vec![false; CELLS * CELLS];
    let mut queue = VecDeque::from([from]);
    seen[from.1 * CELLS + from.0] = true;
    while let Some((x, y)) = queue.pop_front() {
        let neighbours = [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ];
        for (nx, ny) in neighbours {
            if nx < CELLS && ny < CELLS && open[ny * CELLS + nx] && !seen[ny * CELLS + nx] {
                seen[ny * CELLS + nx] = true;
                queue.push_back((nx, ny));
            }
        }
    }
    seen
}

#[test]
fn each_sides_lane_towers_stand_further_from_its_fountain_tier_by_tier() {
    for map in dota_maps() {
        for (team, towers) in [
            (Team::Radiant, map.radiant_towers),
            (Team::Dire, map.dire_towers),
        ] {
            let fountain = hero_spawn_pos(map, team);
            for lane in [rules::LANE_MID, rules::LANE_TOP, rules::LANE_BOT] {
                let reach = |tier: u8| {
                    let found: Vec<Vec2> = towers
                        .iter()
                        .filter(|(l, t, _)| *l == lane && *t == tier)
                        .map(|(_, _, at)| *at)
                        .collect();
                    assert_eq!(found.len(), 1, "{team:?} lane {lane} tier {tier}");
                    distance_squared(found[0], fountain)
                };
                assert!(
                    reach(3) < reach(2) && reach(2) < reach(1),
                    "{team:?} lane {lane}: tiers out of order"
                );
            }
        }
    }
}

#[test]
fn every_camp_spawns_inside_its_box_on_walkable_ground() {
    for map in dota_maps() {
        let field = Clearance::of_map(map);
        for camp in map.camps {
            assert!(camp.spawn_box.contains(camp.pos), "{camp:?}");
            let (x, y) = cell(camp.pos);
            assert!(
                field.terrain().cell_open(x, y),
                "camp at {:?} on closed ground",
                camp.pos
            );
        }
    }
}

#[test]
fn no_tree_stands_on_a_landmark() {
    for map in dota_maps() {
        let mut landmarks: Vec<Vec2> = map.fountains.to_vec();
        landmarks.extend(map.ancients.iter().flatten());
        landmarks.extend(map.radiant_towers.iter().map(|(_, _, at)| *at));
        landmarks.extend(map.dire_towers.iter().map(|(_, _, at)| *at));
        landmarks.extend(
            map.barracks
                .iter()
                .flat_map(|side| side.iter().map(|(_, _, at)| *at)),
        );
        landmarks.extend(map.fillers.iter().flat_map(|side| side.iter().copied()));
        landmarks.extend(map.creep_spawns.iter().flatten());
        landmarks.extend(map.camps.iter().map(|camp| camp.pos));
        landmarks.extend(map.outposts.iter().map(|outpost| outpost.at));
        landmarks.extend(
            map.watchers
                .iter()
                .chain(map.shrines)
                .chain(map.lotus_pools),
        );
        landmarks.extend(map.twin_gates.iter().map(|gate| gate.at));
        landmarks.extend(map.rune_spots.iter().map(|spot| spot.at));
        // A tree's trunk covers no landmark's spot (a Watcher of the 7.41 map stands 90 units
        // from one).
        let clear = i64::from(rules::TREE_RADIUS);
        for tree in tree_positions(map, terrain_of(map)) {
            for &landmark in &landmarks {
                assert!(
                    distance_squared(tree, landmark) >= clear * clear,
                    "a tree at {tree:?} stands on the landmark at {landmark:?}"
                );
            }
        }
    }
}

#[test]
fn each_sides_base_reaches_the_other_and_every_camp_and_rune_over_open_ground() {
    for map in dota_maps() {
        let open = open_cells(map);
        for (from, to) in [(Team::Radiant, Team::Dire), (Team::Dire, Team::Radiant)] {
            let reached = reachable(&open, cell(hero_spawn_pos(map, from)));
            let mut goals = vec![hero_spawn_pos(map, to)];
            goals.extend(map.camps.iter().map(|camp| camp.pos));
            goals.extend(map.rune_spots.iter().map(|spot| spot.at));
            for goal in goals {
                let (x, y) = cell(goal);
                assert!(
                    reached[y * CELLS + x],
                    "{goal:?} cannot be walked to from {from:?}"
                );
            }
        }
    }
}

/// The cells the Dota map's fog walls close, counted and hashed: a regenerated map that closes
/// other cells (a wall node dropped or moved) fails here until the change is looked at and the
/// pin moved with it. Pinned on the 7.41f map, whose hand table closed the same cells.
const FOG_WALL_CELLS: (usize, u64) = (405, 0x78df_874f_5a2f_ef72);

#[test]
fn the_fog_walls_close_the_cells_they_are_pinned_to() {
    let cells = crate::game::fow_wall_cells(terrain_of(map_of(MapId(0))));
    let digest = cells
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &(x, y)| {
            (hash ^ (u64::from(x) << 16 | u64::from(y))).wrapping_mul(0x0100_0000_01b3)
        });
    assert_eq!((cells.len(), digest), FOG_WALL_CELLS);
}

#[test]
fn both_sides_hold_as_many_of_each_landmark() {
    for map in dota_maps() {
        for lane in [rules::LANE_MID, rules::LANE_TOP, rules::LANE_BOT] {
            for tier in 1..=4 {
                let count = |towers: &[(u8, u8, Vec2)]| {
                    towers
                        .iter()
                        .filter(|(l, t, _)| *l == lane && *t == tier)
                        .count()
                };
                assert_eq!(count(map.radiant_towers), count(map.dire_towers));
            }
        }
        assert_eq!(map.barracks[0].len(), map.barracks[1].len());
        assert_eq!(map.fillers[0].len(), map.fillers[1].len());
        let half = |at: Vec2| match at.x.to_int() + at.y.to_int() < rules::MAP_SIZE {
            true => Team::Radiant,
            false => Team::Dire,
        };
        let per_side = |spots: &mut dyn Iterator<Item = Vec2>| {
            let teams: Vec<Team> = spots.map(half).collect();
            (
                teams.iter().filter(|t| **t == Team::Radiant).count(),
                teams.iter().filter(|t| **t == Team::Dire).count(),
            )
        };
        for (what, (radiant, dire)) in [
            ("outposts", per_side(&mut map.outposts.iter().map(|o| o.at))),
            ("shrines", per_side(&mut map.shrines.iter().copied())),
            (
                "lotus pools",
                per_side(&mut map.lotus_pools.iter().copied()),
            ),
            (
                "twin gates",
                per_side(&mut map.twin_gates.iter().map(|g| g.at)),
            ),
        ] {
            assert_eq!(radiant, dire, "{what}");
        }
        for outpost in map.outposts {
            assert_eq!(outpost.home, half(outpost.at));
        }
        let jungle = |team: Team| {
            map.rune_spots
                .iter()
                .filter(|spot| spot.kind == RuneSpotKind::Jungle(team))
                .count()
        };
        assert_eq!(jungle(Team::Radiant), jungle(Team::Dire));
    }
}
