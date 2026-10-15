//! Walking, routes round what stands, and bodies eased apart.

use crate::game::{
    CellGrid, Clearance, Entity, Ground, HEROES, ITEM_BOOTS, MELEE_CREEP, March, UnitOrder, World,
    circle_stops, find_path, isqrt64, map_of, plan_radius, point_along, rules,
};
use bota_proto::{Fixed, HeroId, MapId, SlotId, Team, Vec2};

use super::support::*;

/// A xorshift stream of draws from a seed.
fn xorshift(mut seed: u64) -> impl FnMut() -> u64 {
    move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    }
}

#[test]
fn the_integer_square_root_is_the_floor_of_the_real_one() {
    for n in 0..4096i64 {
        let root = isqrt64(n);
        assert!(root * root <= n, "root squared must not pass n: {n}");
        assert!((root + 1) * (root + 1) > n, "the root must be the floor");
    }
    let mut draw = xorshift(0x1234_5678_9abc_def0);
    for _ in 0..16_384 {
        let n = (draw() >> 1) as i64;
        let root = i128::from(isqrt64(n));
        assert!(root * root <= i128::from(n));
        assert!((root + 1) * (root + 1) > i128::from(n));
    }
    assert_eq!(isqrt64(i64::MAX), 3_037_000_499);
}

#[test]
fn an_order_to_walk_moves_a_body_and_turns_it_first() {
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(1000, 1000));
    world.settle();
    world.set_order(
        hero,
        UnitOrder::Move {
            pos: Vec2::from_ints(0, 1000),
        },
    );
    // Facing east and sent west: it turns before it takes a step.
    let start = world.transform.get(hero).expect("placed").pos;
    world.step();
    assert_eq!(
        world.transform.get(hero).map(|t| t.pos),
        Some(start),
        "the first tick is spent coming round"
    );
    for _ in 0..30 {
        world.step();
    }
    let now = world.transform.get(hero).expect("alive").pos;
    assert!(now.x < start.x, "it walks towards where it was sent");
    // Coming round costs whole ticks, so a turn a little short of the way
    // is the faster start; the line is kept to within that little.
    assert!(
        (now.y.to_int() - start.y.to_int()).abs() < 40,
        "and keeps to the line it was sent along: {now:?}"
    );
}

#[test]
fn a_body_does_not_walk_through_a_building() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let (_, _, tower) = map.radiant_towers[0];
    let hero = hero_at(&mut world, tower - Vec2::from_ints(400, 0));
    world.settle();
    world.set_order(
        hero,
        UnitOrder::Move {
            pos: tower + Vec2::from_ints(400, 0),
        },
    );
    let mut nearest = i64::MAX;
    for _ in 0..300 {
        world.step();
        let at = world.transform.get(hero).expect("alive").pos;
        nearest = nearest.min(isqrt64(at.distance_squared(tower)));
    }
    let hull = world.hull.get(hero).expect("has one").collision;
    assert!(
        nearest > i64::from(hull.raw),
        "it never stood inside the tower"
    );
    let end = world.transform.get(hero).expect("alive").pos;
    assert!(end.x > tower.x, "and it got past all the same: {end:?}");
}

#[test]
fn two_bodies_on_one_spot_are_eased_apart() {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let one = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at);
    let other = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at);
    world.settle();
    for _ in 0..30 {
        world.step();
    }
    let (a, b) = (
        world.transform.get(one).expect("alive").pos,
        world.transform.get(other).expect("alive").pos,
    );
    let hulls = world.hull.get(one).expect("has one").collision
        + world.hull.get(other).expect("has one").collision;
    // Easing apart stops at the moment they stop overlapping, which leaves
    // them touching exactly.
    let apart = isqrt64(a.distance_squared(b));
    assert!(
        apart >= i64::from(hulls.raw),
        "still inside one another: {a:?} {b:?}"
    );
}

#[test]
fn a_creep_that_has_stood_long_enough_shoves_through() {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at);
    world.march.insert(creep, March { next: 0 });
    // A hexagon of bodies packed about it, each just clear of it and of
    // its neighbours, so no step in any direction stays clear of them all.
    for (dx, dy) in [
        (72, 0),
        (36, 63),
        (-36, 63),
        (-72, 0),
        (-36, -63),
        (36, -63),
    ] {
        world.spawn_unit(&MELEE_CREEP, Team::Radiant, at + Vec2::from_ints(dx, dy));
    }
    world.settle();
    let aim = at + Vec2::from_ints(400, 0);
    world.set_order(creep, UnitOrder::AttackMove { pos: aim });
    for _ in 0..rules::MARCH_SHOVE_TICKS - 2 {
        world.step();
    }
    assert_eq!(
        world.transform.get(creep).map(|t| t.pos),
        Some(at),
        "boxed in, it does not move"
    );
    for _ in 0..12 {
        world.step();
    }
    assert_ne!(
        world.transform.get(creep).map(|t| t.pos),
        Some(at),
        "but once it has stood long enough it shoves through"
    );
}

#[test]
fn what_flies_goes_straight_over_what_a_walker_goes_round() {
    let hero_at = Vec2::from_ints(6000, 9216);
    let far = Vec2::from_ints(9000, 9216);
    let mut world = a_world_with_a_wall(hero_at, far);
    let hero = world.seats[0].unit.expect("stood up");
    let courier = the_courier(&world);
    world.transform.get_mut(hero).expect("standing").pos = far;
    world.transform.get_mut(courier).expect("flies").pos = hero_at;
    hand_item(&mut world, courier, ITEM_BOOTS, 0);
    assert!(world.courier_deliver(courier));
    let delivered = |world: &World| {
        world
            .inventory
            .get(hero)
            .is_some_and(|bag| bag.held().count() > 0)
    };
    // Straight there means the line it walks never wanders off the line it
    // was on: a flier routed like a walker swings wide round the wall.
    let mut widest = 0;
    for _ in 0..400 {
        world.step();
        let now = world.transform.get(courier).expect("flies").pos;
        widest = widest.max((now.y.raw - hero_at.y.raw).abs());
        if delivered(&world) {
            break;
        }
    }
    assert!(delivered(&world), "it got there");
    assert!(
        widest < rules::units(200).raw,
        "and it went over the wall rather than round it"
    );
}

#[test]
fn a_way_found_to_a_spot_that_walks_away_is_found_again() {
    let hero_at = Vec2::from_ints(6000, 9216);
    let goal_at = Vec2::from_ints(9000, 9216);
    let mut world = a_world_with_a_wall(hero_at, goal_at);
    let hero = world.seats[0].unit.expect("stood up");
    world.transform.get_mut(hero).expect("standing").pos = hero_at;
    let mut goal = goal_at;
    world.set_order(hero, UnitOrder::Move { pos: goal });
    world.step();
    assert!(
        world
            .route
            .get(hero)
            .is_some_and(|route| !route.corners.is_empty()),
        "a way round the wall was found"
    );
    // The spot creeps away, a little every tick, exactly as a walking hero
    // does to a courier chasing it. Never enough in one tick to look like a
    // new goal, and after a while far beyond where the way was laid to.
    for _ in 0..60 {
        goal += Vec2::from_ints(0, 20);
        world.set_order(hero, UnitOrder::Move { pos: goal });
        world.step();
    }
    let end = world
        .route
        .get(hero)
        .and_then(|route| route.corners.last().copied())
        .expect("still walking a way round");
    assert!(
        end.within(goal, rules::units(600)),
        "the way it walks leads where the spot is now, not where it was"
    );
}

#[test]
fn a_hero_told_to_walk_into_a_tower_walks_up_to_it_and_stands() {
    let mut world = a_match_with_nobody_in_it(7, MapId(0));
    let (_, _, tower) = rules::RADIANT_TOWERS[2];
    let start = Vec2::from_ints(4000, 4500);
    let hero = hero_at(&mut world, start);
    world.set_order(hero, UnitOrder::Move { pos: tower });
    let mut seen = Vec::new();
    for _ in 0..(10 * rules::TICKS_PER_SECOND) {
        world.advance(&[]);
        seen.push(world.transform.get(hero).expect("standing").pos);
    }
    let footprint = plan_radius(rules::units(rules::TOWER_COLLISION));
    let last = *seen.last().expect("walked");
    assert!(
        !last.within(start, rules::units(300)),
        "it set off: {last:?}"
    );
    assert!(
        !last.within(tower, footprint),
        "it never stood in the tower"
    );
    assert!(
        last.within(tower, footprint + rules::units(rules::GRID_CELL_SIZE * 2)),
        "and got as near as the ground lets it: {last:?}"
    );
    let settled = &seen[seen.len() - rules::TICKS_PER_SECOND as usize..];
    assert!(
        settled.iter().all(|at| *at == last),
        "and stood there through the last second instead of circling"
    );
}

#[test]
fn a_walk_to_where_no_way_leads_ends_at_the_nearest_spot_got_to() {
    let mut cells = CellGrid::open();
    let wall = 100;
    for cy in 0..rules::GRID_CELLS {
        cells.close_cell(wall, cy);
    }
    let grid = Clearance::from_cells(cells);
    let from = Vec2::from_ints(1000, 1000);
    let beyond = Vec2::from_ints(10000, 1000);
    let body = rules::units(rules::HERO_COLLISION);
    let path = find_path(&grid, from, beyond, body);
    let end = *path.last().expect("it walks somewhere");
    assert!(
        end.x.to_int() < wall as i32 * rules::GRID_CELL_SIZE,
        "it stops this side of the wall: {end:?}"
    );
    assert!(
        end.x.to_int() >= (wall as i32 - 1) * rules::GRID_CELL_SIZE,
        "right up against it: {end:?}"
    );
    // A spot that can be stood on and reached is the walk's own end.
    let there = Vec2::from_ints(5000, 3000);
    assert_eq!(find_path(&grid, from, there, body).last(), Some(&there));
}

#[test]
fn a_capsule_is_stopped_by_a_circle_exactly_where_the_circle_stops_it() {
    let at = Vec2::from_ints(3000, 4000);
    let theirs = Fixed::from_int(48);
    let field = Clearance::build(CellGrid::open(), vec![(at, theirs)]);
    let radius = Fixed::from_int(8);
    let mut draw = xorshift(0x0dd0_5eed_1234_5678);
    let mut stopped = 0;
    let mut free = 0;
    for _ in 0..4096 {
        let from = Vec2::from_ints(
            (draw() % rules::MAP_SIZE as u64) as i32,
            (draw() % rules::MAP_SIZE as u64) as i32,
        );
        let to = Vec2::from_ints(
            (draw() % rules::MAP_SIZE as u64) as i32,
            (draw() % rules::MAP_SIZE as u64) as i32,
        );
        let reference = !field
            .circles()
            .iter()
            .any(|&(centre, their)| circle_stops(centre, their, from, to, radius, true));
        assert_eq!(
            field.capsule_clear(from, to, radius),
            reference,
            "from {from:?} to {to:?}"
        );
        if reference { free += 1 } else { stopped += 1 }
    }
    assert!(stopped > 0 && free > 0, "both answers have to be seen");
}

#[test]
fn the_demo_lake_is_walled_by_its_shore_and_crossed_at_its_ford() {
    let map = map_of(MapId(1));
    let ground = Ground::of(map);
    let cell = |x: i32, y: i32| {
        (
            (x / rules::GRID_CELL_SIZE) as usize,
            (y / rules::GRID_CELL_SIZE) as usize,
        )
    };
    // The lane's ford: shallow water, walked through.
    let (fx, fy) = cell(8768, 9024);
    assert!(ground.cell_walkable(fx, fy), "the ford is walked");
    assert!(
        ground.water(Vec2::from_ints(8768, 9024)),
        "and it is water underfoot"
    );
    // The rocks ringing the lake bar the way outside the openings.
    for (x, y) in [(7968, 8864), (8032, 8928)] {
        let (cx, cy) = cell(x, y);
        assert!(
            !ground.cell_walkable(cx, cy),
            "the shore at ({x},{y}) is not walked over"
        );
    }
    // The fountain structures close their own ground too.
    for at in map.fountains {
        let (cx, cy) = cell(at.x.to_int(), at.y.to_int());
        assert!(
            !ground.cell_walkable(cx, cy),
            "a fountain is stood beside, not inside"
        );
    }
}

#[test]
fn walkers_and_marchers_both_work_round_a_wall_of_bodies() {
    let wall = |world: &mut World, x: i32, y: i32| {
        for i in -2..=2i32 {
            world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(x, y + i * 40));
        }
    };
    // A hero ordered through the knot gets round it briskly.
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    wall(&mut world, 5400, 5000);
    world.settle();
    let goal = Vec2::from_ints(5900, 5000);
    world.set_order(hero, UnitOrder::Move { pos: goal });
    let took = ticks_until_near(&mut world, hero, goal, 130);
    assert!(took.is_some(), "the hero works round the wall");
    // A marching creep held off any lane does the same.
    let mut world = World::new();
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5000, 6000));
    world.march.insert(creep, March { next: 0 });
    wall(&mut world, 5400, 6000);
    world.settle();
    let goal = Vec2::from_ints(5900, 6000);
    world.set_order(creep, UnitOrder::AttackMove { pos: goal });
    let took = ticks_until_near(&mut world, creep, goal, 130);
    assert!(took.is_some(), "the creep works round the wall");
}

#[test]
fn a_hero_touching_a_tower_and_sent_past_it_goes_round_briskly() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let (_, _, tower) = map.radiant_towers[0];
    let touching = rules::TOWER_COLLISION + rules::HERO_COLLISION + 1;
    let hero = hero_at(&mut world, tower - Vec2::from_ints(touching, 0));
    world.settle();
    let goal = tower + Vec2::from_ints(300, 0);
    world.set_order(hero, UnitOrder::Move { pos: goal });
    let took = ticks_until_near(&mut world, hero, goal, 200);
    assert!(
        took.is_some_and(|t| t <= 90),
        "round the tower and past it within three seconds, not {took:?}"
    );
}

#[test]
fn a_hero_walks_round_a_wave_stood_fighting() {
    let mut world = the_demo_lane_at_its_first_wave();
    for _ in 0..(12 * rules::TICKS_PER_SECOND) {
        world.step();
    }
    let creeps: Vec<Entity> = world
        .entities
        .iter()
        .filter(|e| world.march.get(*e).is_some())
        .collect();
    assert!(
        creeps.len() >= 6,
        "the waves have met and are still standing"
    );
    let (mut cx, mut cy) = (0i64, 0i64);
    for creep in &creeps {
        let at = world.transform.get(*creep).expect("standing").pos;
        cx += i64::from(at.x.to_int());
        cy += i64::from(at.y.to_int());
    }
    let centre = Vec2::from_ints(
        (cx / creeps.len() as i64) as i32,
        (cy / creeps.len() as i64) as i32,
    );
    let from = centre - Vec2::from_ints(400, 300);
    let goal = centre + Vec2::from_ints(400, 300);
    let hero = hero_at(&mut world, from);
    world.settle();
    world.set_order(hero, UnitOrder::Move { pos: goal });
    let took = ticks_until_near(&mut world, hero, goal, 240);
    assert!(
        took.is_some_and(|t| t <= 150),
        "past the fight within five seconds, not {took:?}"
    );
}

/// A wave marching up its lane finds a hero of its own side standing on
/// the road and walks round it without stopping.
#[test]
fn marchers_walk_round_a_hero_standing_on_their_lane() {
    let mut world = the_demo_lane_at_its_first_wave();
    let route = world.walked_lanes()[0][0].clone();
    let spot = point_along(route[1], route[2], rules::units(300));
    hero_at(&mut world, spot);
    world.settle();
    let creeps = radiant_marchers(&world);
    let blocked = lane_progress(&route, spot);
    // On the way up to the hero and past it, no creep stands stalled for
    // long: the enemy wave it meets further on is another matter.
    for _ in 0..(7 * rules::TICKS_PER_SECOND) {
        world.step();
        for creep in &creeps {
            let at = world.transform.get(*creep).expect("alive").pos;
            if lane_progress(&route, at) > blocked + 150 {
                continue;
            }
            let stalled = world.motion.get(*creep).expect("walks").stalled;
            assert!(
                stalled < 20,
                "a creep stood stalled {stalled} ticks against the hero at ({},{})",
                at.x.to_int(),
                at.y.to_int()
            );
        }
    }
    for creep in &creeps {
        let at = world.transform.get(*creep).expect("alive").pos;
        assert!(
            lane_progress(&route, at) > blocked + 150,
            "a creep is held up by the hero at ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
    }
}

/// Creep blocking: a hero pacing just ahead of its own wave's lead creep is
/// run into, and holds that creep back against a wave left alone.
#[test]
fn a_hero_pacing_before_a_wave_holds_the_creep_it_covers() {
    let ticks = 10 * rules::TICKS_PER_SECOND;
    let mut world = the_demo_lane_at_its_first_wave();
    let route = world.walked_lanes()[0][0].clone();
    for _ in 0..30 {
        world.step();
    }
    let (start, start_mean) = wave_progress(&world, &route);
    for _ in 0..ticks {
        world.step();
    }
    let (_, free_mean) = wave_progress(&world, &route);
    // The same wave again, with a hero set down just ahead of it.
    let mut world = the_demo_lane_at_its_first_wave();
    for _ in 0..30 {
        world.step();
    }
    let (front, _) = wave_progress(&world, &route);
    let hero = hero_at(&mut world, along_lane(&route, front + 70, 0));
    world.settle();
    let mut contacts = 0;
    for _ in 0..ticks {
        // Keep just ahead of the creep furthest up the lane.
        let hp = world.transform.get(hero).expect("standing").pos;
        let mine = lane_progress(&route, hp);
        let creep = lead_creep_at(&world, &route).unwrap_or(hp);
        let dir = along_lane(&route, mine + 100, 0) - along_lane(&route, mine, 0);
        let target = creep + point_along(Vec2::ZERO, dir, rules::units(70));
        world.set_order(hero, UnitOrder::Move { pos: target });
        world.step();
        contacts += world
            .entities
            .iter()
            .filter(|e| {
                world.march.get(*e).is_some()
                    && world.motion.get(*e).is_some_and(|m| m.bumped == world.tick)
            })
            .count();
    }
    let held = radiant_marchers(&world)
        .into_iter()
        .map(|e| lane_progress(&route, world.transform.get(e).expect("standing").pos) - start)
        .min()
        .expect("the wave stands");
    assert!(
        contacts >= 3,
        "the creeps ran into the hero: {contacts} contacts"
    );
    assert!(
        held + 150 < free_mean - start_mean,
        "the creep it covered is held back: {held} against a free wave's {}",
        free_mean - start_mean
    );
}

/// Where the Radiant lane creep furthest along a route stands.
fn lead_creep_at(world: &World, route: &[Vec2]) -> Option<Vec2> {
    let mut lead: Option<(i64, Vec2)> = None;
    for creep in radiant_marchers(world) {
        let at = world.transform.get(creep).expect("standing").pos;
        let progress = lane_progress(route, at);
        if lead.is_none_or(|(had, _)| progress > had) {
            lead = Some((progress, at));
        }
    }
    lead.map(|(_, at)| at)
}

#[test]
fn every_hero_sent_at_a_tower_from_afar_walks_into_reach_and_strikes() {
    for id in 0..HEROES.len() as u16 {
        let map = map_of(MapId(0));
        let mut world = World::on_map(map);
        let tower_at = rules::RADIANT_TOWERS[0].2;
        let tower = structure_at(&world, tower_at);
        let hero = world.spawn_hero(
            Team::Dire,
            tower_at + Vec2::from_ints(900, 900),
            SlotId(0),
            HeroId(id),
        );
        world.settle();
        world.fill_pools(hero);
        let before = world.health.get(tower).expect("standing").hp;
        world.set_order(
            hero,
            UnitOrder::Attack {
                target: tower,
                last_seen: tower_at,
            },
        );
        let struck = (0..8 * rules::TICKS_PER_SECOND).any(|_| {
            world.step();
            world.health.get(tower).expect("standing").hp < before
        });
        assert!(
            struck,
            "hero {id} never struck the tower within eight seconds"
        );
    }
}

/// Spots among Map2's trees a hero walked into at its own size, where the
/// route's margin finds no way out: one whose node has no room for the
/// margin, one whose nodes with room for it are closed in.
#[test]
fn a_hero_among_trees_too_close_for_the_route_margin_still_walks_home() {
    let map = map_of(MapId(2));
    for (team, x, y) in [
        (Team::Dire, 739_766_649, 791_609_959),
        (Team::Radiant, 469_633_835, 567_213_857),
    ] {
        let mut world = World::on_map(map);
        let start = Vec2 {
            x: Fixed { raw: x },
            y: Fixed { raw: y },
        };
        let hero = world.spawn_hero(team, start, SlotId(0), HeroId(1));
        world.settle();
        let home = map.fountains[usize::from(team == Team::Dire)];
        world.set_order(hero, UnitOrder::Move { pos: home });
        for _ in 0..(5 * rules::TICKS_PER_SECOND) {
            world.step();
        }
        let now = world.transform.get(hero).expect("standing").pos;
        let gained = isqrt64(start.distance_squared(home)) - isqrt64(now.distance_squared(home));
        assert!(
            gained > i64::from(rules::units(500).raw),
            "from {start:?} it walked towards home, not stood at {now:?}"
        );
    }
}
