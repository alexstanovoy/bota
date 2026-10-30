//! A felled tree is a gap in the forest: walkers go through it, and again
//! round it once the tree has grown back.
//!
//! Dota's long pather treats trees as static blockers on the gridnav, and a
//! tree cut down opens its gridnav cells (Liquipedia, *Pathfinding*; *Trees*).
//! Dota publishes nothing on a unit already walking when a tree falls; bota
//! lays the walk again from where the unit stands (see
//! [`World::lay_passability`]), and the second test pins that choice.

use bota_core::{isqrt64, map_of, rules};
use bota_proto::{ItemId, ItemSlot, MapId, Order, SlotId, Target, Team, Vec2};

use super::support::{hand_item, seat_hero};
use crate::game::{
    Clearance, Command, Entity, Obstacles, Planner, World, plan_radius, terrain_of, tree_positions,
};

/// A tree with open ground either side of it along a line, and the spots
/// the tests walk between.
struct Gap {
    tree: Vec2,
    /// One side of the tree, 500 off it.
    a: Vec2,
    /// The other side.
    b: Vec2,
    /// Where the one who fells the tree stands, beside it.
    feller: Vec2,
    /// Where the feller goes after, well off the way.
    away: Vec2,
}

/// The least the gap must save a walk, in units.
const GAIN: i64 = 80;

fn hero_room() -> bota_proto::Fixed {
    plan_radius(rules::units(rules::HERO_COLLISION))
}

/// A spot in sixteenths of a unit (the assertions measure, they do not
/// simulate).
fn xy(at: Vec2) -> (i64, i64) {
    let shift = bota_proto::Fixed::FRAC_BITS - 4;
    (i64::from(at.x.raw) >> shift, i64::from(at.y.raw) >> shift)
}

/// The distance between two spots in sixteenths of a unit.
fn fine(a: (i64, i64), b: (i64, i64)) -> i64 {
    isqrt64((a.0 - b.0).pow(2) + (a.1 - b.1).pow(2))
}

fn dist(a: Vec2, b: Vec2) -> i32 {
    (fine(xy(a), xy(b)) / 16) as i32
}

/// The length of a walk's corners from where it starts.
fn length(from: Vec2, way: &[Vec2]) -> i64 {
    let (mut last, mut total) = (from, 0);
    for &corner in way {
        total += i64::from(dist(last, corner));
        last = corner;
    }
    total
}

/// How near a walk's corners come to a spot.
fn nearest(from: Vec2, way: &[Vec2], to: Vec2) -> i32 {
    let (mut last, mut best) = (from, i32::MAX);
    for &corner in way {
        let (a, b, t) = (xy(last), xy(corner), xy(to));
        for i in 0..=40 {
            let at = (a.0 + (b.0 - a.0) * i / 40, a.1 + (b.1 - a.1) * i / 40);
            best = best.min((fine(at, t) / 16) as i32);
        }
        last = corner;
    }
    best
}

/// The first tree beside the Dire mid tower that stands in a line of
/// trees: with it down, the long pather's way between two spots either side
/// of it is at least [`GAIN`] shorter and goes by it; found on the real forest by
/// the long pather itself.
fn a_tree_in_a_line() -> Gap {
    let map = map_of(MapId(2));
    let base = Clearance::of_map(map);
    let radius = rules::units(rules::TREE_RADIUS);
    let room = hero_room();
    let collision = rules::units(rules::HERO_COLLISION);
    let mut best: Option<(i64, Gap)> = None;
    for tree in tree_positions(map, terrain_of(map)) {
        let mut open = base.clone();
        open.set_circles(
            base.circles()
                .iter()
                .copied()
                .filter(|circle| *circle != (tree, radius))
                .collect(),
        );
        for (dx, dy) in [
            (500, 0),
            (0, 500),
            (354, 354),
            (354, -354),
            (300, 0),
            (0, 300),
            (212, 212),
            (212, -212),
        ] {
            let off = Vec2::from_ints(dx, dy);
            let (a, b) = (tree - off, tree + off);
            let (feller, away) = (
                tree + Vec2::from_ints(-dy / 4, dx / 4),
                tree + Vec2::from_ints(-dy * 3 / 2, dx * 3 / 2),
            );
            if ![a, b, feller, away]
                .iter()
                .all(|at| open.fits_at(*at, room))
            {
                continue;
            }
            let round = crate::game::find_path(&base, a, b, collision);
            let gap = crate::game::find_path(&open, a, b, collision);
            if round.last() != Some(&b) || gap.last() != Some(&b) {
                continue;
            }
            let gain = length(a, &round) - length(a, &gap);
            if gain >= GAIN
                && nearest(a, &gap, tree) < 30
                && nearest(a, &round, tree) > 60
                && best.as_ref().is_none_or(|(most, _)| gain > *most)
            {
                best = Some((
                    gain,
                    Gap {
                        tree,
                        a,
                        b,
                        feller,
                        away,
                    },
                ));
            }
        }
    }
    best.expect("no tree of the forest stands in a line").1
}

/// A world on the solo map with a walker (seat 0) at `a` and a feller
/// (seat 1) with a Tango beside the tree.
fn forest(gap: &Gap) -> (World, Entity, Entity) {
    let mut world = World::on_map(map_of(MapId(2)));
    let walker = seat_hero(&mut world, 0, Team::Radiant, gap.a);
    let feller = seat_hero(&mut world, 1, Team::Radiant, gap.feller);
    // The feller's bag is seat 1's.
    hand_item(&mut world, feller, ItemId::TANGO, 3);
    if let Some(stack) = world
        .inventory
        .get_mut(feller)
        .and_then(|bag| bag.slots[0].as_mut())
    {
        stack.owner = SlotId(1);
    }
    world.settle();
    world.step();
    (world, walker, feller)
}

fn order(world: &mut World, slot: u8, order: Order) {
    world.advance(&[Command {
        slot: SlotId(slot),
        unit: None,
        order,
        queue: false,
        seq: 0,
    }]);
}

fn move_to(world: &mut World, slot: u8, at: Vec2) {
    order(
        world,
        slot,
        Order::Move {
            target: Target::Pos(at),
        },
    );
}

/// What a walk came to: how far the walker went, and how near it came to
/// the tree.
struct Walk {
    way: i64,
    nearest: i32,
}

/// Steps the world until the walker has arrived, noting its way.
fn arrive(world: &mut World, walker: Entity, tree: Vec2, to: Vec2, mut walk: Walk) -> Walk {
    let mut last = world.transform.get(walker).expect("stands").pos;
    for _ in 0..40 * rules::TICKS_PER_SECOND {
        if last.within(to, rules::units(30)) {
            return walk;
        }
        world.step();
        let now = world.transform.get(walker).expect("stands").pos;
        walk.way += i64::from(dist(last, now));
        walk.nearest = walk.nearest.min(dist(now, tree));
        last = now;
    }
    panic!("the walker never got to {to:?}");
}

fn walk(world: &mut World, walker: Entity, tree: Vec2, to: Vec2) -> Walk {
    move_to(world, 0, to);
    let start = Walk {
        way: 0,
        nearest: i32::MAX,
    };
    arrive(world, walker, tree, to, start)
}

/// The feller eats the tree by the real item order.
fn eat(world: &mut World, gap: &Gap) {
    order(
        world,
        1,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::Pos(gap.tree),
        },
    );
    for _ in 0..60 {
        if world.trees.felled().count() > 0 {
            return;
        }
        world.step();
    }
    panic!("the tango did not fell the tree");
}

/// Round the line of trees, a Tango fells one, and the next walk takes the
/// gap: measurably shorter, and through the felled tree's spot.
#[test]
fn a_walker_takes_the_gap_a_tango_made_in_a_line_of_trees() {
    let gap = a_tree_in_a_line();
    let (mut world, walker, _) = forest(&gap);
    let round = walk(&mut world, walker, gap.tree, gap.b);
    assert!(round.nearest > 60, "round the tree, {} off", round.nearest);
    eat(&mut world, &gap);
    let through = walk(&mut world, walker, gap.tree, gap.a);
    assert!(
        through.nearest < 64,
        "through the gap, {} off the tree",
        through.nearest
    );
    assert!(
        through.way + GAIN < round.way,
        "gap {} against round {}",
        through.way,
        round.way
    );
}

/// A walker already walking round when the tree falls has its walk laid
/// again from where it stands and takes the gap. Dota says nothing of a
/// walk in progress when a tree is cut (no source found); bota's choice is
/// that the gridnav is the ground as it stands, so the long path is asked
/// again at once.
#[test]
fn a_walker_on_its_way_round_turns_into_the_gap_when_the_tree_falls() {
    let gap = a_tree_in_a_line();
    let (mut world, walker, _) = forest(&gap);
    let round = walk(&mut world, walker, gap.tree, gap.b);
    // Back to the start the long way round, to know the round way's length.
    let home = walk(&mut world, walker, gap.tree, gap.a);
    assert!(home.nearest > 60);
    move_to(&mut world, 0, gap.b);
    let mut last = world.transform.get(walker).expect("stands").pos;
    let mut walked = 0;
    for _ in 0..4 {
        world.step();
        let now = world.transform.get(walker).expect("stands").pos;
        walked += i64::from(dist(last, now));
        last = now;
    }
    eat(&mut world, &gap);
    let rest = arrive(
        &mut world,
        walker,
        gap.tree,
        gap.b,
        Walk {
            way: walked,
            nearest: i32::MAX,
        },
    );
    assert!(
        rest.nearest < 64,
        "turned into the gap, {} off",
        rest.nearest
    );
    assert!(
        rest.way + GAIN < round.way,
        "{} against {}",
        rest.way,
        round.way
    );
}

/// Three minutes on, with nobody within 150 of the spot, the tree is back
/// and the walk goes round again.
#[test]
fn the_gap_closes_when_the_tree_grows_back() {
    let gap = a_tree_in_a_line();
    let (mut world, walker, _) = forest(&gap);
    let round = walk(&mut world, walker, gap.tree, gap.b);
    eat(&mut world, &gap);
    let through = walk(&mut world, walker, gap.tree, gap.a);
    assert!(through.nearest < 64);
    move_to(&mut world, 1, gap.away);
    for _ in 0..rules::TREE_REGROW_TICKS + 2 * rules::TICKS_PER_SECOND {
        world.step();
    }
    assert_eq!(world.trees.felled().count(), 0, "the tree is back");
    let again = walk(&mut world, walker, gap.tree, gap.b);
    assert!(again.nearest > 60, "round again, {} off", again.nearest);
    assert!(
        (again.way - round.way).abs() * 20 < round.way,
        "{} against {}",
        again.way,
        round.way
    );
}

/// The long pather's field is the world's own, with the room laid fresh for
/// what stands now: the same planner finds the gap after the fall and the
/// round way after the regrowth, and its field is what a field laid from
/// scratch over the same circles would be.
#[test]
fn the_long_pather_reads_the_ground_as_it_now_stands() {
    let gap = a_tree_in_a_line();
    let (mut world, _, _) = forest(&gap);
    let collision = rules::units(rules::HERO_COLLISION);
    let mut planner = Planner::new();
    let mut way = |world: &World| {
        let ob = Obstacles {
            field: &world.clearance,
            extra: &[],
        };
        let corners = planner.find_path(&ob, gap.a, gap.b, collision);
        (length(gap.a, &corners), nearest(gap.a, &corners, gap.tree))
    };
    let (round, round_by) = way(&world);
    assert!(round_by > 60);
    eat(&mut world, &gap);
    let (through, through_by) = way(&world);
    assert!(
        through_by < 30 && through + GAIN <= round,
        "{through} {round}"
    );
    // A field laid from scratch over the circles standing is the same.
    let fresh = Clearance::build(
        world.clearance.terrain().clone(),
        world.clearance.circles().to_vec(),
    );
    let ob = Obstacles {
        field: &fresh,
        extra: &[],
    };
    let again = Planner::new().find_path(&ob, gap.a, gap.b, collision);
    assert_eq!(length(gap.a, &again), through);
    for dx in (-200..=200).step_by(8) {
        for dy in (-200..=200).step_by(8) {
            let at = gap.tree + Vec2::from_ints(dx, dy);
            assert_eq!(
                world.clearance.fits_at(at, hero_room()),
                fresh.fits_at(at, hero_room()),
                "{at:?}"
            );
        }
    }
    move_to(&mut world, 1, gap.away);
    for _ in 0..rules::TREE_REGROW_TICKS + 2 * rules::TICKS_PER_SECOND {
        world.step();
    }
    let (closed, closed_by) = way(&world);
    assert_eq!((closed, closed_by > 60), (round, true));
}
