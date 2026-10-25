//! Walks through the forest, held to the shortest way Dota's own gridnav
//! has through it.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bota_core::{isqrt64, map_of, rules};
use bota_proto::{Fixed, HeroId, MapId, SlotId, Team, Vec2};

use crate::game::{Clearance, UnitOrder, World, plan_radius, terrain_of, tree_positions};

/// Sixteenths of a world unit: the reference way is worked out in them.
const FINE: i64 = 16;
/// One gridnav cell, in sixteenths.
const CELL: i64 = rules::GRID_CELL_SIZE as i64 * FINE;
const CELLS: usize = rules::GRID_CELLS;

/// A spot in sixteenths of a unit.
type Spot = (i64, i64);

fn spot(at: Vec2) -> Spot {
    let shift = Fixed::FRAC_BITS - 4;
    (i64::from(at.x.raw) >> shift, i64::from(at.y.raw) >> shift)
}

fn dist(a: Spot, b: Spot) -> i64 {
    isqrt64((b.0 - a.0).pow(2) + (b.1 - a.1).pow(2))
}

/// Dota's gridnav for a map: 64-unit cells (dota.fgd `@gridnav(64, ...)`),
/// each closed by the terrain or by a tree's 128x128 square, the four cells
/// about the cell corner it stands on (Liquipedia, *Trees*).
struct Gridnav {
    closed: Vec<bool>,
}

impl Gridnav {
    fn of(map: &'static bota_core::MapDef) -> Gridnav {
        let field = Clearance::of_map(map);
        let mut closed = vec![false; CELLS * CELLS];
        for cy in 0..CELLS {
            for cx in 0..CELLS {
                closed[cy * CELLS + cx] = !field.terrain().cell_open(cx, cy);
            }
        }
        for tree in tree_positions(map, terrain_of(map)) {
            let (tx, ty) = (tree.x.to_int() / 64, tree.y.to_int() / 64);
            for (cx, cy) in [(tx - 1, ty - 1), (tx, ty - 1), (tx - 1, ty), (tx, ty)] {
                closed[cy as usize * CELLS + cx as usize] = true;
            }
        }
        Gridnav { closed }
    }

    fn cell_open(&self, cx: i64, cy: i64) -> bool {
        let inside = |c: i64| (0..CELLS as i64).contains(&c);
        inside(cx) && inside(cy) && !self.closed[cy as usize * CELLS + cx as usize]
    }

    /// Whether a point may stand on a spot: every cell it touches is open,
    /// so a spot on the corner where two closed cells meet is shut.
    fn open(&self, (x, y): Spot) -> bool {
        let cells = |v: i64| {
            let own = v.div_euclid(CELL);
            [
                own,
                if v.rem_euclid(CELL) == 0 {
                    own - 1
                } else {
                    own
                },
            ]
        };
        cells(x)
            .iter()
            .all(|&cx| cells(y).iter().all(|&cy| self.cell_open(cx, cy)))
    }

    /// Whether a point walks the segment through open cells alone.
    fn sees(&self, a: Spot, b: Spot) -> bool {
        let steps = (dist(a, b) / (2 * FINE)).max(1);
        (0..=steps)
            .all(|i| self.open((a.0 + (b.0 - a.0) * i / steps, a.1 + (b.1 - a.1) * i / steps)))
    }

    /// The length of the shortest way a point has between two spots through
    /// open cells, at any angle: Theta* over cell centres.
    fn shortest(&self, a: Spot, b: Spot) -> Option<i64> {
        if self.sees(a, b) {
            return Some(dist(a, b));
        }
        let start = (a.1 / CELL) as usize * CELLS + (a.0 / CELL) as usize;
        let at = |i: usize| {
            let centre = |c: usize| c as i64 * CELL + CELL / 2;
            if i == start {
                a
            } else {
                (centre(i % CELLS), centre(i / CELLS))
            }
        };
        let mut g = vec![i64::MAX; CELLS * CELLS];
        let mut parent = vec![start; CELLS * CELLS];
        let mut open = BinaryHeap::new();
        g[start] = 0;
        open.push(Reverse((dist(a, b), start)));
        let mut best: Option<i64> = None;
        while let Some(Reverse((_, i))) = open.pop() {
            if best.is_some_and(|best| g[i] + dist(at(i), b) >= best) {
                break;
            }
            let p = parent[i];
            if self.sees(at(i), b) {
                let way = if self.sees(at(p), b) {
                    g[p] + dist(at(p), b)
                } else {
                    g[i] + dist(at(i), b)
                };
                best = Some(best.map_or(way, |best| best.min(way)));
                continue;
            }
            for (dx, dy) in [
                (1, 0),
                (0, 1),
                (-1, 0),
                (0, -1),
                (1, 1),
                (-1, 1),
                (-1, -1),
                (1, -1),
            ] {
                let (x, y) = ((i % CELLS) as i64 + dx, (i / CELLS) as i64 + dy);
                if !self.cell_open(x, y) {
                    continue;
                }
                let j = y as usize * CELLS + x as usize;
                if !self.sees(at(i), at(j)) {
                    continue;
                }
                let (way, from) = if self.sees(at(p), at(j)) {
                    (g[p] + dist(at(p), at(j)), p)
                } else {
                    (g[i] + dist(at(i), at(j)), i)
                };
                if way < g[j] {
                    (g[j], parent[j]) = (way, from);
                    open.push(Reverse((way + dist(at(j), b), j)));
                }
            }
        }
        best
    }
}

/// How far a hero walks from one spot to another when ordered there, in
/// sixteenths, or None when it does not get there within twenty seconds.
fn walked(map: &'static bota_core::MapDef, from: Vec2, to: Vec2) -> Option<i64> {
    let mut world = World::on_map(map);
    let hero = world.spawn_hero(Team::Dire, from, SlotId(0), HeroId::SHADOW_FIEND);
    world.settle();
    world.set_order(hero, UnitOrder::Move { pos: to });
    let mut way = 0;
    let mut last = from;
    for _ in 0..20 * rules::TICKS_PER_SECOND {
        world.step();
        let now = world.transform.get(hero).expect("standing").pos;
        way += dist(spot(last), spot(now));
        last = now;
        if now.within(to, rules::units(8)) {
            return Some(way);
        }
    }
    None
}

/// A hero sent across the forest beside the Dire mid tier-1 tower, from one
/// side of it to the other, walks about as short a way as Dota's own gridnav
/// has there (Liquipedia, *Pathfinding*: the pather "always makes a unit walk
/// the shortest distance path"): through the lanes one cell wide between its
/// trees, not round the clumps they part.
///
/// Measured over these walks: walked / gridnav shortest 0.96 on average,
/// 1.00 at most (bota's trees are round, Dota's square: a walk round one
/// is a little shorter). Before the walking lattice had nodes on the
/// gridnav's cell centres, the first of them walked 1.11 times the way.
#[test]
fn a_hero_walks_through_the_forest_by_the_dire_mid_tower_the_shortest_way() {
    let map = map_of(MapId(2));
    let gridnav = Gridnav::of(map);
    let field = Clearance::of_map(map);
    let trees = tree_positions(map, terrain_of(map));
    let room = plan_radius(rules::units(rules::HERO_COLLISION));
    let tower = Vec2::from_ints(9740, 9868);
    let in_forest = |at: Vec2| {
        field.fits_at(at, room)
            && gridnav.open(spot(at))
            && trees
                .iter()
                .filter(|tree| tree.within(at, rules::units(300)))
                .count()
                >= 4
    };
    let mut seed = 0x1234_5678_9abc_def1u64;
    let mut draw = || loop {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let at = Vec2::from_ints(
            tower.x.to_int() - 1400 + (seed % 2800) as i32,
            tower.y.to_int() - 1400 + ((seed >> 32) % 2800) as i32,
        );
        if in_forest(at) {
            return at;
        }
    };
    let (mut walks, mut total, mut shortest) = (0, 0, 0);
    while walks < 16 {
        let (from, to) = (draw(), draw());
        let apart = dist(spot(from), spot(to)) / FINE;
        if !(500..=1800).contains(&apart) || gridnav.sees(spot(from), spot(to)) {
            continue;
        }
        let Some(best) = gridnav.shortest(spot(from), spot(to)) else {
            continue;
        };
        let way =
            walked(map, from, to).unwrap_or_else(|| panic!("{from:?} to {to:?}: never got there"));
        assert!(
            way * 100 <= best * 108,
            "{from:?} to {to:?}: walked {}, the gridnav's way {}",
            way / FINE,
            best / FINE
        );
        (walks, total, shortest) = (walks + 1, total + way, shortest + best);
    }
    assert!(
        total <= shortest,
        "walked {total} in all, the gridnav's ways {shortest}"
    );
}
