//! Walking, routes round what stands, and bodies eased apart.

use bota_core::{CellGrid, HEROES, MELEE_CREEP, RANGED_CREEP, isqrt64, map_of, point_along, rules};

use crate::game::{
    Clearance, Entity, Ground, March, UnitOrder, World, circle_stops, find_path, plan_radius,
};
use bota_proto::{Fixed, HeroId, ItemId, MapId, SlotId, Team, Vec2};

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
    run(&mut world, 30);
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
    run(&mut world, 30);
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
fn what_flies_goes_straight_over_what_a_walker_goes_round() {
    let hero_at = Vec2::from_ints(6000, 9216);
    let far = Vec2::from_ints(9000, 9216);
    let mut world = a_world_with_a_wall(hero_at, far);
    let hero = world.seats[0].unit.expect("stood up");
    let courier = raise_owner(&mut world, rules::COURIER_FLYING_LEVEL);
    world.transform.get_mut(hero).expect("standing").pos = far;
    world.transform.get_mut(courier).expect("flies").pos = hero_at;
    hand_item(&mut world, courier, ItemId::BOOTS, 0);
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
    let ground = Ground::of(crate::game::terrain_of(map));
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

/// A walker sent through a knot of standing bodies goes round it, a hero
/// and a marching lane creep alike: stationary units are blockers to every
/// unit's short pather (Liquipedia, *Pathfinding*).
#[test]
fn a_hero_and_a_marcher_go_round_a_wall_of_standing_bodies() {
    let wall = |world: &mut World, x: i32, y: i32| {
        for i in -2..=2i32 {
            world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(x, y + i * 40));
        }
    };
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    wall(&mut world, 5400, 5000);
    world.settle();
    let goal = Vec2::from_ints(5900, 5000);
    world.set_order(hero, UnitOrder::Move { pos: goal });
    let took = ticks_until_near(&mut world, hero, goal, 130);
    assert!(took.is_some(), "the hero works round the wall");
    let mut world = World::new();
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5000, 6000));
    world.march.insert(creep, March { next: 0 });
    wall(&mut world, 5400, 6000);
    world.settle();
    let goal = Vec2::from_ints(5900, 6000);
    world.set_order(creep, UnitOrder::AttackMove { pos: goal });
    let took = ticks_until_near(&mut world, creep, goal, 130);
    assert!(took.is_some(), "the marcher works round the wall");
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
    run(&mut world, 12 * rules::TICKS_PER_SECOND);
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
    // Past it, or fighting the enemy wave: a ranged creep may stand
    // shooting from behind it.
    for creep in &creeps {
        let at = world.transform.get(*creep).expect("alive").pos;
        assert!(
            lane_progress(&route, at) > blocked + 150 || world.target_of(*creep).is_some(),
            "a creep is held up by the hero at ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
    }
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
        let hero = world.spawn_hero(team, start, SlotId(0), HeroId::SHADOW_FIEND);
        world.settle();
        let home = map.fountains[usize::from(team == Team::Dire)];
        world.set_order(hero, UnitOrder::Move { pos: home });
        run(&mut world, 5 * rules::TICKS_PER_SECOND);
        let now = world.transform.get(hero).expect("standing").pos;
        let gained = isqrt64(start.distance_squared(home)) - isqrt64(now.distance_squared(home));
        assert!(
            gained > i64::from(rules::units(500).raw),
            "from {start:?} it walked towards home, not stood at {now:?}"
        );
    }
}

/// Map 2's first Radiant wave walked up its lane until its lead creep is
/// `at` along the lane, with the route and the lead creep.
fn map2_wave_led_to(at: i64) -> (World, Vec<Vec2>, Entity) {
    let mut world = World::on_map(map_of(MapId(2)));
    while world.tick < rules::FIRST_WAVE_TICK {
        world.step();
    }
    let route = world.walked_lanes()[0][0].clone();
    loop {
        world.step();
        let lead = radiant_marchers(&world)
            .into_iter()
            .max_by_key(|e| (lane_progress(&route, pos_of(&world, *e)), *e))
            .expect("a wave came out");
        if lane_progress(&route, pos_of(&world, lead)) >= at {
            return (world, route, lead);
        }
    }
}

fn pos_of(world: &World, entity: Entity) -> Vec2 {
    world.transform.get(entity).expect("standing").pos
}

/// The spot just in front of a creep, touching it, the way it walks.
fn head_on(world: &World, creep: Entity) -> Vec2 {
    let at = pos_of(world, creep);
    let aim = world
        .route
        .get(creep)
        .map(|route| route.corners.first().copied().unwrap_or(route.end))
        .expect("a route");
    let touch = rules::HERO_COLLISION + rules::MELEE_CREEP_COLLISION + 1;
    point_along(at, aim, rules::units(touch))
}

/// A hero that stops dead in front of the lead creep of a marching wave is
/// a stationary unit, which every walker's short pather lays its way round
/// (Liquipedia, *Pathfinding*), so the wave walks round it rather than
/// queueing behind it for good: "stand in front of a creep [and] he will try
/// to pass on your left/right side" (Gaming StackExchange q/65482).
#[test]
fn a_wave_walks_round_a_hero_who_stops_before_its_lead_creep() {
    let (mut world, route, lead) = map2_wave_led_to(2500);
    let spot = head_on(&world, lead);
    let hero = hero_at(&mut world, spot);
    world.set_order(hero, UnitOrder::Stand);
    let blocked = lane_progress(&route, spot);
    let creeps = radiant_marchers(&world);
    let mut lead_past = None;
    let mut all_past = None;
    for tick in 1..=(5 * rules::TICKS_PER_SECOND) {
        world.step();
        let past = |e: &Entity| lane_progress(&route, pos_of(&world, *e)) > blocked + 60;
        if lead_past.is_none() && past(&lead) {
            lead_past = Some(tick);
        }
        if creeps.iter().all(past) {
            all_past = Some(tick);
            break;
        }
    }
    // Measured on this map: the lead creep past in 43 ticks (1.4 s): its
    // first blocked wait, then two on the creep that follows it, which
    // stands in its way round the hero's west side; the wave in 52 (1.7 s).
    // Dota's guides give no number, only that the blocker must move again
    // "before they can run around you".
    assert!(
        lead_past.is_some_and(|t| t <= 3 * rules::TICKS_PER_SECOND / 2),
        "the lead creep walks round within a second and a half, not {lead_past:?}"
    );
    assert!(
        all_past.is_some_and(|t| t <= 3 * rules::TICKS_PER_SECOND),
        "the wave walks round within three seconds, not {all_past:?}"
    );
}

/// Where the first clash of map 2's first waves happens: the Radiant lead
/// creep's lane progress and the tick it first takes an enemy, with a hero
/// keeping just in front of it the whole way when `block` says so.
fn first_clash(block: bool) -> (i64, u32) {
    first_clash_tapping(block, None)
}

fn first_clash_tapping(block: bool, taps: Option<(u32, u32)>) -> (i64, u32) {
    const LEAD_BY: i32 = 200;
    let (mut world, route, lead) = map2_wave_led_to(300);
    let start = head_on(&world, lead);
    let hero = block.then(|| hero_at(&mut world, start));
    world.settle();
    for _ in 0..(60 * rules::TICKS_PER_SECOND) {
        let lead_now = radiant_marchers(&world)
            .into_iter()
            .max_by_key(|e| (lane_progress(&route, pos_of(&world, *e)), *e))
            .expect("the wave stands");
        if let Some(hero) = hero {
            // Keep walking on just ahead of the lead creep, on its line, never
            // arriving: a blocker that stops is walked round.
            let lead_at = pos_of(&world, lead_now);
            let ahead = point_along(lead_at, head_on(&world, lead_now), rules::units(LEAD_BY));
            let stopped = taps.is_some_and(|(every, stop)| world.tick % every < stop);
            if stopped {
                world.set_order(hero, UnitOrder::Stand);
            } else {
                world.set_order(hero, UnitOrder::Move { pos: ahead });
            }
        }
        world.step();
        if world.target_of(lead).is_some() || world.target_of(lead_now).is_some() {
            return (lane_progress(&route, pos_of(&world, lead_now)), world.tick);
        }
    }
    panic!("the waves never met");
}

/// A hero that keeps walking just ahead of its first wave's lead creep, on
/// the creep's line, all the way to the clash, holds that creep to its own
/// pace: the creep walks against a moving body and is held, as Dota's lane
/// creeps are (Liquipedia, *Unit Size*), and the waves meet nearer the own
/// side. Measured: the clash 220 units nearer and 20 ticks (0.67 s) later
/// than an unblocked wave's. A straight walk can do little more, the hero
/// (310) being only a little slower than the creep (325); a block that weaves
/// across the creep's line holds it longer.
#[test]
fn a_hero_who_keeps_blocking_delays_the_wave() {
    let (free_at, free_tick) = first_clash(false);
    let (held_at, held_tick) = first_clash(true);
    assert!(
        held_at + 100 <= free_at,
        "the clash is nearer the own side: {held_at} against {free_at}"
    );
    assert!(
        held_tick >= free_tick + 10,
        "and later: tick {held_tick} against {free_tick}"
    );
}

/// A hero walking down its lane straight into its own marching wave, dead on
/// the lead creep's line, gets through: the two meet square and pass each
/// other on the right.
#[test]
fn a_hero_walking_head_on_into_its_wave_gets_through() {
    let (mut world, route, lead) = map2_wave_led_to(1500);
    let spot = point_along(
        head_on(&world, lead),
        pos_of(&world, lead),
        rules::units(-400),
    );
    let hero = hero_at(&mut world, spot);
    let goal = along_lane(&route, lane_progress(&route, pos_of(&world, lead)) - 600, 0);
    world.settle();
    world.set_order(hero, UnitOrder::Move { pos: goal });
    let took = ticks_until_near(&mut world, hero, goal, 600);
    // Measured: 114 ticks for about 1000 units against the whole wave.
    assert!(
        took.is_some_and(|t| t <= 6 * rules::TICKS_PER_SECOND),
        "through the wave within six seconds, not {took:?}"
    );
}

/// A hero pressed against its outer tower while its own wave marches past it, sent on down the
/// lane with the wave or back up against it, walks on sliding along the creeps it meets, as a
/// player's unit does in Dota, instead of standing until the whole wave has gone by. The spot,
/// the tower and the creeps are those of a hero seen stuck in a match.
#[test]
fn a_hero_in_its_marching_wave_by_its_tower_walks_on() {
    for (goal, within) in [
        (Vec2::from_ints(9515, 9624), 60),
        (Vec2::from_ints(10300, 10200), 70),
    ] {
        let mut world = World::on_map(map_of(MapId(2)));
        let hero = world.spawn_hero(
            Team::Dire,
            Vec2::from_ints(9899, 9790),
            SlotId(1),
            HeroId::SHADOW_FIEND,
        );
        for (def, x, y) in [
            (&MELEE_CREEP, 9874, 9700),
            (&MELEE_CREEP, 9946, 9745),
            (&MELEE_CREEP, 9796, 9666),
            (&RANGED_CREEP, 10023, 9817),
            (&MELEE_CREEP, 10100, 9900),
            (&MELEE_CREEP, 10160, 9960),
        ] {
            world.spawn_creep(def, Team::Dire, Vec2::from_ints(x, y), rules::LANE_MID, 0);
        }
        world.settle();
        world.set_order(hero, UnitOrder::Move { pos: goal });
        let mut still = 0;
        let mut took = None;
        for tick in 1..=within {
            let before = world.transform.get(hero).expect("standing").pos;
            world.step();
            let at = world.transform.get(hero).expect("standing").pos;
            still += u32::from(before == at);
            if at.within(goal, rules::units(50)) {
                took = Some(tick);
                break;
            }
        }
        // Measured: 12 and 1 ticks stood, the creeps it meets waited on
        // before it slides along them (Dota's blocked waits).
        assert!(took.is_some(), "to {goal:?} within {within} ticks");
        assert!(
            still <= 3 * rules::UNIT_BLOCKED_WAIT_TICKS[0],
            "stood {still} ticks on the way to {goal:?}"
        );
    }
}

/// Two rows of trees along the x axis from 4800 to 5600 about y = 5000,
/// leaving a way `width` units wide between them.
fn tree_corridor(world: &mut World, width: i32) {
    let half = width / 2 + rules::TREE_RADIUS;
    for x in (4800..=5600).step_by(80) {
        world.trees.plant(Vec2::from_ints(x, 5000 + half), u32::MAX);
        world.trees.plant(Vec2::from_ints(x, 5000 - half), u32::MAX);
    }
    world.lay_sight_block();
    world.lay_passability();
}

/// Two heroes sent through a way between trees from either end meet head on
/// inside it. Where it has room for both side by side they pass each other.
/// Where it has not, neither can step on: each stands pressed against the
/// other, a stationary unit to the other's short pather (Liquipedia,
/// *Pathfinding*), so they do not push for good but find their way round,
/// one of them the long way round the trees.
#[test]
fn two_heroes_meeting_between_trees_get_through() {
    let (west, east) = (Vec2::from_ints(4600, 5000), Vec2::from_ints(5800, 5000));
    for width in [70, 100, 130] {
        let mut world = World::new();
        tree_corridor(&mut world, width);
        let one = hero_at(&mut world, Vec2::from_ints(4700, 5000));
        let other = world.spawn_hero(
            Team::Dire,
            Vec2::from_ints(5700, 5000),
            SlotId(1),
            HeroId::SHADOW_FIEND,
        );
        world.settle();
        world.set_order(one, UnitOrder::Move { pos: east });
        world.set_order(other, UnitOrder::Move { pos: west });
        let mut through = (None, None);
        for tick in 1..=(25 * rules::TICKS_PER_SECOND) {
            world.step();
            if through.0.is_none() && pos_of(&world, one).within(east, rules::units(50)) {
                through.0 = Some(tick);
            }
            if through.1.is_none() && pos_of(&world, other).within(west, rules::units(50)) {
                through.1 = Some(tick);
            }
        }
        // Measured, both heroes, 1100 units each (108 ticks in the open):
        // 220 and 114 ticks at 70 wide, 242 and 146 at 100, 249 and 158 at
        // 130. Neither holds the other for good: a meeting is waited on
        // (Dota's blocked waits), then walked round.
        assert!(
            through.0.is_some_and(|t| t <= 22 * rules::TICKS_PER_SECOND)
                && through.1.is_some_and(|t| t <= 22 * rules::TICKS_PER_SECOND),
            "both through a {width}-wide way, not {through:?}"
        );
    }
}

/// A hero standing in a way between trees too narrow to pass it is walked
/// round the long way, whether the way is one hull wide or more: the
/// standing hero closes the way like the trees do, and the short pather
/// traces round both, keeping them in mind until it is past.
#[test]
fn a_hero_standing_in_a_way_between_trees_is_walked_round() {
    let goal = Vec2::from_ints(5800, 5000);
    for width in [70, 130] {
        let mut world = World::new();
        tree_corridor(&mut world, width);
        let walker = hero_at(&mut world, Vec2::from_ints(4700, 5000));
        let stander = world.spawn_hero(
            Team::Dire,
            Vec2::from_ints(5200, 5000),
            SlotId(1),
            HeroId::SHADOW_FIEND,
        );
        world.settle();
        world.set_order(stander, UnitOrder::Stand);
        world.set_order(walker, UnitOrder::Move { pos: goal });
        let took = ticks_until_near(&mut world, walker, goal, 10 * rules::TICKS_PER_SECOND);
        // Measured: 181 and 183 ticks, in and out again and round.
        assert!(
            took.is_some_and(|t| t <= 7 * rules::TICKS_PER_SECOND),
            "round the stander in a {width}-wide way within seven seconds, not {took:?}"
        );
    }
}

/// A hero walking down the lane straight into the enemy's marching wave gets
/// through it: the creeps that turn on it stand to strike and are walked
/// round, those still marching are slid past.
#[test]
fn a_hero_walking_into_the_enemy_wave_gets_through() {
    let (mut world, route, lead) = map2_wave_led_to(1500);
    let spot = point_along(
        head_on(&world, lead),
        pos_of(&world, lead),
        rules::units(-400),
    );
    let hero = world.spawn_hero(Team::Dire, spot, SlotId(1), HeroId::SHADOW_FIEND);
    let goal = along_lane(&route, lane_progress(&route, pos_of(&world, lead)) - 600, 0);
    world.settle();
    world.set_order(hero, UnitOrder::Move { pos: goal });
    let took = ticks_until_near(&mut world, hero, goal, 10 * rules::TICKS_PER_SECOND);
    // Measured: 113 ticks for about 1000 units.
    assert!(
        took.is_some_and(|t| t <= 6 * rules::TICKS_PER_SECOND),
        "through the enemy wave within six seconds, not {took:?}"
    );
}

/// A hero walking into the creeps of a fight on the lane, head on against a
/// creep that walks into it from the other side to get at an enemy behind
/// it, gets past: neither can step on, so each stands, a stationary unit
/// the other's short pather lays its way round (Liquipedia, *Pathfinding*),
/// rather than both pushing for good. The spot is a hero's seen pushing
/// against such a knot in a match for seconds on end.
#[test]
fn a_hero_and_a_creep_walking_into_each_other_in_a_fight_get_past() {
    let mut world = World::on_map(map_of(MapId(2)));
    let hero = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(8987, 9102),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    let standing = [
        (Team::Dire, 9046, 9076),
        (Team::Dire, 9131, 8996),
        (Team::Radiant, 9015, 8956),
    ]
    .map(|(team, x, y)| world.spawn_unit(&MELEE_CREEP, team, Vec2::from_ints(x, y)));
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(9021, 9159));
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(8947, 8992));
    world.settle();
    let goal = Vec2::from_ints(9128, 9243);
    let mut took = None;
    for tick in 1..=(5 * rules::TICKS_PER_SECOND) {
        for unit in standing.into_iter().chain([mark]) {
            world.set_order(unit, UnitOrder::Hold);
        }
        let last_seen = pos_of(&world, mark);
        world.set_order(
            creep,
            UnitOrder::Attack {
                target: mark,
                last_seen,
            },
        );
        world.set_order(hero, UnitOrder::Move { pos: goal });
        world.step();
        if pos_of(&world, hero).within(goal, rules::units(50)) {
            took = Some(tick);
            break;
        }
    }
    // Measured: 30 ticks for 200 units.
    assert!(
        took.is_some_and(|t| t <= 2 * rules::TICKS_PER_SECOND),
        "past the creep within two seconds, not {took:?}"
    );
}

/// A hero sent to a spot inside a knot of creeps standing to fight walks up
/// to the knot and ends its walk there, as near as it gets: the creeps are
/// stationary units, which its short pather takes in like the ground
/// (Liquipedia, *Pathfinding*), rather than pushing against them for good.
/// The spots are a hero's seen pushing against such a knot in a match.
#[test]
fn a_hero_sent_into_a_knot_of_standing_creeps_ends_its_walk_beside_it() {
    let mut world = World::on_map(map_of(MapId(2)));
    let start = Vec2::from_ints(8962, 8914);
    let hero = world.spawn_hero(Team::Radiant, start, SlotId(0), HeroId::SHADOW_FIEND);
    let knot = [
        (Team::Radiant, 8886, 9151),
        (Team::Radiant, 8899, 9072),
        (Team::Dire, 9014, 9125),
        (Team::Dire, 9022, 9045),
        (Team::Dire, 8988, 8982),
    ]
    .map(|(team, x, y)| world.spawn_unit(&MELEE_CREEP, team, Vec2::from_ints(x, y)));
    world.settle();
    let goal = Vec2::from_ints(8962, 9114);
    world.set_order(hero, UnitOrder::Move { pos: goal });
    let mut over = None;
    for tick in 1..=(3 * rules::TICKS_PER_SECOND) {
        for creep in knot {
            world.set_order(creep, UnitOrder::Hold);
        }
        world.step();
        if over.is_none() && world.route.get(hero).is_some_and(|route| route.done) {
            over = Some(tick);
        }
    }
    let at = pos_of(&world, hero);
    // Measured: over in 24 ticks, 76 units from the spot (200 at the start).
    assert!(
        over.is_some_and(|t| t <= 2 * rules::TICKS_PER_SECOND),
        "the walk ends, not {over:?}"
    );
    assert!(
        at.within(goal, rules::units(120)),
        "beside the knot, not at {at:?}"
    );
}

/// A hero walking into the side of a creep that walks across its way waits
/// on it the first blocked wait (Dota's `dota_unit_blocked_wait_time`) and
/// then slides along the creep's edge without halting again: the slide is
/// the bodies pushing against each other, not a way the hero turns to, so it
/// keeps facing where it walks and keeps stepping.
#[test]
fn a_hero_meeting_a_creep_walking_across_its_way_waits_once_and_slides_on() {
    let mut world = World::new();
    let hero = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5150, 4880));
    world.settle();
    let goal = Vec2::from_ints(5600, 5000);
    world.set_order(hero, UnitOrder::Move { pos: goal });
    world.set_order(
        creep,
        UnitOrder::Move {
            pos: Vec2::from_ints(5150, 5600),
        },
    );
    // Facing the way already, the hero sets off at once.
    run(&mut world, 2);
    let (mut touching, mut halted) = (0, 0);
    for _ in 0..(2 * rules::TICKS_PER_SECOND) {
        let before = pos_of(&world, hero);
        world.step();
        let after = pos_of(&world, hero);
        let reach = rules::HERO_COLLISION + rules::MELEE_CREEP_COLLISION + 4;
        if after.within(pos_of(&world, creep), rules::units(reach)) {
            touching += 1;
            if after == before {
                halted += 1;
            }
        }
        if after.within(goal, rules::units(50)) {
            break;
        }
    }
    // Measured: 6 ticks in contact, 5 of them waited (the first wait), none
    // halted after it.
    assert!(touching > 0, "the two met");
    assert!(
        halted <= rules::UNIT_BLOCKED_WAIT_TICKS[0],
        "the hero halted {halted} of {touching} ticks in contact"
    );
}

/// A blocker that walks just ahead of its first wave's lead creep, on its
/// line, and taps Stop as Dota's guides teach (for 0.05-0.2 s every
/// 0.25-0.5 s: dotaforall's creep-block guide, CreepBlocker.lua) holds the
/// creep each time it is stopped in front of it: the creep waits on a body
/// that blocks it before it goes round (`dota_unit_blocked_wait_time`,
/// 0.15 s, then 0.3 and 0.4), and the blocker walks on before that. Over the
/// guides' rhythms the waves meet on average more than twice as much nearer
/// the own side as behind a blocker that only walks ahead. Measured: walking
/// ahead 220 units; tapping, 115-1259 by rhythm, 526 on average, the guides'
/// "just outside tier 1 range" (some 550-600 units on this lane).
#[test]
fn a_blocker_tapping_stop_holds_the_wave_as_dota_guides_teach() {
    let (free_at, _) = first_clash(false);
    let (walked_at, _) = first_clash(true);
    let walked = free_at - walked_at;
    let mut gains = Vec::new();
    for every in [8u32, 10, 12, 15] {
        for stop in [2u32, 3, 4, 5, 6] {
            let (at, _) = first_clash_tapping(true, Some((every, stop)));
            gains.push(free_at - at);
        }
    }
    let mean = gains.iter().sum::<i64>() / gains.len() as i64;
    assert!(
        mean >= 2 * walked,
        "tapping holds the wave {mean} units on average, walking {walked}: {gains:?}"
    );
}

/// A hero pressed against the ground by the mid river, with three enemy creeps standing to fight
/// it in a row on its other side, sent past them walks out through the way left between the
/// creeps and the ground, though it is narrower than the walking lattice, and round them: a
/// standing body is planned round in continuous space, as the short pather traces it
/// (Liquipedia, *Pathfinding*), not only where the lattice has room. The spots are a hero's
/// seen standing there for nine seconds in a match.
#[test]
fn a_hero_hemmed_in_by_standing_creeps_and_the_ground_walks_out() {
    let mut world = World::on_map(map_of(MapId(2)));
    let raw = |x: i32, y: i32| Vec2 {
        x: Fixed { raw: x },
        y: Fixed { raw: y },
    };
    let hero = world.spawn_hero(
        Team::Radiant,
        raw(562_028_139, 547_192_589),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    let row = [
        raw(556_034_549, 545_918_891),
        raw(556_279_858, 555_559_308),
        raw(558_996_760, 550_790_587),
    ]
    .map(|at| world.spawn_unit(&MELEE_CREEP, Team::Dire, at));
    world.settle();
    let goal = Vec2::from_ints(8608, 7805);
    world.set_order(hero, UnitOrder::Move { pos: goal });
    let mut took = None;
    for tick in 1..=(10 * rules::TICKS_PER_SECOND) {
        for creep in row {
            world.set_order(creep, UnitOrder::Hold);
        }
        world.step();
        if pos_of(&world, hero).within(goal, rules::units(50)) {
            took = Some(tick);
            break;
        }
    }
    // Measured: 60 ticks for some 560 units (54 in the open); before the
    // short pather traced walls, it stood there for good.
    assert!(
        took.is_some_and(|t| t <= 6 * rules::TICKS_PER_SECOND),
        "out and to the spot within six seconds, not {took:?}"
    );
}
