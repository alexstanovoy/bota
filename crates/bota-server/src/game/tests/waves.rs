//! Lane waves, their routes, and the structures that open lanes.

use crate::game::rules;
use crate::game::{Entity, World};
use bota_proto::Fixed;

use super::support::*;

#[test]
fn a_wave_carries_one_flag_from_the_fifth_wave_on() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    let fifth = rules::FIRST_WAVE_TICK + 4 * rules::WAVE_PERIOD_TICKS;
    while world.tick < fifth {
        world.step();
    }
    let flags = world
        .entities
        .iter()
        .filter(|e| world.kind.get(*e) == Some(&bota_proto::UnitKind::CreepFlagbearer))
        .count();
    assert_eq!(flags, 2 * usize::from(map.lanes), "one a lane a side");
}

/// The rule this guards: a creep that left its route, chasing or pushed,
/// is sent on to the route ahead of it, never back to where it left.
#[test]
fn a_creep_off_its_route_rejoins_it_ahead_and_never_walks_back() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_WAVE_TICK {
        world.step();
    }
    let creep = world
        .entities
        .iter()
        .find(|e| {
            world.march.get(*e).is_some() && world.team.get(*e) == Some(&bota_proto::Team::Radiant)
        })
        .expect("a wave came out");
    let route = world.walked_lanes()[0][0].clone();
    assert!(route.len() >= 3, "the lane has corners to be ahead of");
    // Carried past its next two waypoints and off to one side of the road.
    let ahead = route[2] + bota_proto::Vec2::from_ints(0, 300);
    if let Some(at) = world.transform.get_mut(creep) {
        at.pos = ahead;
    }
    let was = lane_progress(&route, ahead);
    world.step();
    let Some(crate::game::UnitOrder::AttackMove { pos }) =
        world.orders.get(creep).map(|o| o.current)
    else {
        panic!("it is sent somewhere");
    };
    assert!(
        lane_progress(&route, pos) + 40 >= was,
        "it is sent on, not back: to {} from {}",
        lane_progress(&route, pos),
        was
    );
    for _ in 0..90 {
        world.step();
    }
    let now = world.transform.get(creep).expect("alive").pos;
    assert!(
        lane_progress(&route, now) > was,
        "and it gets further along the lane"
    );
}

#[test]
fn a_creep_is_sent_one_way_at_a_time() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_WAVE_TICK {
        world.step();
    }
    let creep = world
        .entities
        .iter()
        .find(|e| {
            world.march.get(*e).is_some() && world.team.get(*e) == Some(&bota_proto::Team::Radiant)
        })
        .expect("a wave came out");
    // Pushed off the route: the mark it is given must not change from tick
    // to tick while nothing about it changes.
    if let Some(at) = world.transform.get_mut(creep) {
        at.pos += bota_proto::Vec2::from_ints(-200, 200);
    }
    world.step();
    let first = world.orders.get(creep).map(|o| o.current);
    world.step();
    let second = world.orders.get(creep).map(|o| o.current);
    assert_eq!(first, second, "it is not pulled two ways in one breath");
    let was = world.transform.get(creep).expect("alive").pos;
    for _ in 0..60 {
        world.step();
    }
    let later = world.transform.get(creep).expect("alive").pos;
    assert!(
        !later.within(was, rules::units(40)),
        "and it actually goes somewhere: {was:?} then {later:?}"
    );
}

/// The bug this guards against: a waypoint that routes a wave around its own
/// tower was cleared straight through the tower, and one Radiant mid creep of
/// every wave spent fifteen seconds wrestling its own tier three.
#[test]
fn no_creep_of_the_first_waves_is_left_wrestling_its_own_base() {
    let cfg = crate::game::MatchConfig {
        match_id: 7,
        master_key: [0; 32],
        picks: vec![],
        map: bota_proto::MapId(0),
        tick_rate: 30,
        mode: bota_proto::TickMode::Realtime,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    for _ in 0..=rules::FIRST_WAVE_TICK {
        world.advance(&[]);
    }
    let first: Vec<(Entity, bota_proto::Vec2)> = world
        .entities
        .iter()
        .filter(|e| world.march.get(*e).is_some())
        .map(|e| {
            let team = world.team.get(e).copied().expect("has a side");
            let lane = world.lane.get(e).copied().expect("has a lane");
            let spawn = crate::game::creep_spawn_pos(world.map, team, lane.0);
            (e, spawn)
        })
        .collect();
    assert_eq!(
        first.len(),
        24,
        "four creeps a lane, three lanes, two sides"
    );
    for _ in 0..(30 * rules::TICKS_PER_SECOND) {
        world.advance(&[]);
    }
    // Half a minute in, everything still standing has long left its base;
    // what died, died out on the lane.
    for (creep, spawn) in &first {
        if !world.entities.iter().any(|e| e == *creep) {
            continue;
        }
        let at = world.transform.get(*creep).expect("standing").pos;
        assert!(
            !at.within(*spawn, rules::units(1500)),
            "a creep is still beside its spawner at ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
    }
    // And the mid waves in particular met in the middle of no-man's land,
    // four against four.
    let meet = bota_proto::Vec2::from_ints(8706, 8838);
    for (creep, _) in &first {
        if !world.entities.iter().any(|e| e == *creep)
            || world.lane.get(*creep).copied() != Some(crate::game::Lane(rules::LANE_MID))
        {
            continue;
        }
        let at = world.transform.get(*creep).expect("standing").pos;
        assert!(
            at.within(meet, rules::units(900)),
            "a mid creep never reached the river: ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
    }
}

/// The bug this guards against: the lane routes were laid once with every
/// tower's footprint in them, and a wave kept walking round the ground a
/// fallen tower had stood on.
#[test]
fn a_wave_walks_over_where_its_tower_stood_once_it_has_fallen() {
    let cfg = crate::game::MatchConfig {
        match_id: 7,
        master_key: [0; 32],
        picks: vec![],
        map: bota_proto::MapId(0),
        tick_rate: 30,
        mode: bota_proto::TickMode::Realtime,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    let (lane, _, tower) = rules::RADIANT_TOWERS[2];
    assert_eq!(lane, rules::LANE_MID, "the Radiant mid tier three");
    let footprint = crate::game::plan_radius(rules::units(rules::TOWER_COLLISION));
    assert!(
        world.walked_lanes()[0][0]
            .iter()
            .all(|spot| !spot.within(tower, footprint)),
        "standing, the tower is walked round"
    );
    fell_at(&mut world, tower);
    assert!(
        world.walked_lanes()[0][0].contains(&tower),
        "fallen, the route runs over its ground"
    );
    for _ in 0..=rules::FIRST_WAVE_TICK {
        world.advance(&[]);
    }
    let mut nearest = i64::MAX;
    for _ in 0..(20 * rules::TICKS_PER_SECOND) {
        world.advance(&[]);
        for creep in world.entities.iter() {
            if world.march.get(creep).is_none()
                || world.team.get(creep) != Some(&bota_proto::Team::Radiant)
                || world.lane.get(creep).map(|l| l.0) != Some(rules::LANE_MID)
            {
                continue;
            }
            let at = world.transform.get(creep).expect("standing").pos;
            nearest = nearest.min(crate::game::isqrt64(at.distance_squared(tower)) >> 16);
        }
    }
    assert!(
        nearest < i64::from(rules::TOWER_COLLISION),
        "the wave walks where the tower's body was: nearest {nearest}"
    );
}

#[test]
fn a_lane_opens_tower_by_tower_into_its_barracks() {
    let mut world = World::on_map(crate::game::map_of(bota_proto::MapId(0)));
    let t1 = rules::RADIANT_TOWERS[0].2;
    let t2 = rules::RADIANT_TOWERS[1].2;
    let t3 = rules::RADIANT_TOWERS[2].2;
    let melee_rax = rules::RADIANT_BARRACKS[0].2;
    let ranged_rax = rules::RADIANT_BARRACKS[1].2;
    assert!(open_at(&world, t1), "the first tower is open from the horn");
    assert!(!open_at(&world, t2), "the second waits on the first");
    assert!(!open_at(&world, t3), "the third waits on the second");
    assert!(
        !open_at(&world, melee_rax),
        "the barracks wait on the third"
    );
    fell_at(&mut world, t1);
    assert!(open_at(&world, t2), "the first fallen opens the second");
    assert!(!open_at(&world, t3), "and only the second");
    fell_at(&mut world, t2);
    assert!(open_at(&world, t3));
    assert!(!open_at(&world, melee_rax), "the barracks still wait");
    fell_at(&mut world, t3);
    assert!(open_at(&world, melee_rax), "the third fallen opens both");
    assert!(open_at(&world, ranged_rax));
}

#[test]
fn a_destroyed_structure_reopens_the_ground_it_blocked() {
    let mut world = World::on_map(crate::game::map_of(bota_proto::MapId(0)));
    let tower = rules::RADIANT_TOWERS[0].2;
    let body = rules::units(rules::HERO_COLLISION);
    assert!(
        world.clearance.walkable(tower),
        "the ground under a tower is ground all the same"
    );
    assert!(
        !world.clearance.stands_clear(tower),
        "but nothing is put down where a standing tower is"
    );
    assert!(
        !world.clearance.fits_at(tower, body),
        "and no walk is planned through it"
    );

    fell_at(&mut world, tower);

    assert!(
        world.clearance.stands_clear(tower) && world.clearance.fits_at(tower, body),
        "the tower's ground is anybody's after its destruction"
    );
}

#[test]
fn the_ancient_waits_for_both_tier_fours() {
    let mut world = World::on_map(crate::game::map_of(bota_proto::MapId(0)));
    let t4_near = rules::RADIANT_TOWERS[9].2;
    let t4_far = rules::RADIANT_TOWERS[10].2;
    let ancient = rules::RADIANT_ANCIENT_POS;
    assert!(
        !open_at(&world, t4_near),
        "the tier fours wait on a broken lane"
    );
    fell_at(&mut world, rules::RADIANT_TOWERS[3].2);
    fell_at(&mut world, rules::RADIANT_TOWERS[4].2);
    fell_at(&mut world, rules::RADIANT_TOWERS[5].2);
    assert!(open_at(&world, t4_near), "any tier three fallen opens them");
    assert!(open_at(&world, t4_far));
    assert!(!open_at(&world, ancient), "the Ancient stands guarded");
    fell_at(&mut world, t4_near);
    assert!(
        !open_at(&world, ancient),
        "one tier four fallen is not enough"
    );
    fell_at(&mut world, t4_far);
    assert!(open_at(&world, ancient), "both fallen open the Ancient");
}

#[test]
fn a_fallen_barracks_turns_the_waves_against_it_super_and_all_of_them_mega() {
    let cfg = crate::game::MatchConfig {
        match_id: 9,
        master_key: [0; 32],
        picks: vec![],
        map: bota_proto::MapId(0),
        tick_rate: 30,
        mode: bota_proto::TickMode::Realtime,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    // Whole barracks: plain waves.
    let plain = next_mid_wave(&mut world, bota_proto::Team::Dire);
    assert!(
        plain
            .iter()
            .all(|e| world.stats.get(*e).expect("settled").max_hp
                <= Fixed::from_int(rules::MELEE_CREEP_HP)),
        "no barracks down, nothing spawns super"
    );
    // The Radiant mid melee barracks falls: Dire mid melee go super, the
    // ranged stay plain, and Radiant's own creeps are untouched.
    fell_at(&mut world, rules::RADIANT_BARRACKS[0].2);
    let dire = next_mid_wave(&mut world, bota_proto::Team::Dire);
    let hp_of = |world: &World, e: Entity| world.stats.get(e).expect("settled").max_hp;
    assert!(
        dire.iter().any(
            |e| hp_of(&world, *e) >= Fixed::from_int(rules::SUPER_MELEE_HP)
                && world.kind.get(*e) == Some(&bota_proto::UnitKind::CreepMelee)
        ),
        "the melee spawn super"
    );
    assert!(
        dire.iter()
            .filter(|e| world.kind.get(**e) == Some(&bota_proto::UnitKind::CreepRanged))
            .all(|e| hp_of(&world, *e) < Fixed::from_int(rules::SUPER_RANGED_HP)),
        "the ranged do not"
    );
    let radiant = next_mid_wave(&mut world, bota_proto::Team::Radiant);
    assert!(
        radiant
            .iter()
            .all(|e| hp_of(&world, *e) <= Fixed::from_int(rules::MELEE_CREEP_HP)),
        "losing a barracks strengthens nobody's own creeps"
    );
    // Every Radiant barracks falls: Dire's waves go mega everywhere.
    for (_, _, at) in rules::RADIANT_BARRACKS.iter().skip(1) {
        fell_at(&mut world, *at);
    }
    let mega = next_mid_wave(&mut world, bota_proto::Team::Dire);
    let mega_melee = mega
        .iter()
        .find(|e| world.kind.get(**e) == Some(&bota_proto::UnitKind::CreepMelee))
        .expect("a melee creep spawned");
    assert_eq!(
        world.stats.get(*mega_melee).expect("settled").attack_time,
        rules::MEGA_MELEE_ATTACK_TIME,
        "a mega melee swings faster than a super one"
    );
}

#[test]
fn the_demo_waves_march_out_and_meet_between_the_towers() {
    let cfg = crate::game::MatchConfig {
        match_id: 11,
        master_key: [0; 32],
        picks: vec![],
        map: bota_proto::MapId(1),
        tick_rate: 30,
        mode: bota_proto::TickMode::Realtime,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    for _ in 0..=rules::FIRST_WAVE_TICK {
        world.advance(&[]);
    }
    let first: Vec<(Entity, bota_proto::Vec2)> = world
        .entities
        .iter()
        .filter(|e| world.march.get(*e).is_some())
        .map(|e| {
            let team = world.team.get(e).copied().expect("has a side");
            (e, crate::game::creep_spawn_pos(world.map, team, 0))
        })
        .collect();
    assert_eq!(first.len(), 8, "one wave a side on the one lane");
    for _ in 0..(30 * rules::TICKS_PER_SECOND) {
        world.advance(&[]);
    }
    // Half a minute in the survivors are grinding in the middle of the
    // lane, nobody is left wrestling its own base, and with no Ancient
    // standing there is nothing to win by.
    let meet = bota_proto::Vec2::from_ints(8850, 9020);
    for (creep, spawn) in &first {
        if !world.entities.iter().any(|e| e == *creep) {
            continue;
        }
        let at = world.transform.get(*creep).expect("standing").pos;
        assert!(
            !at.within(*spawn, rules::units(600)),
            "a creep is still beside its spawner at ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
        assert!(
            at.within(meet, rules::units(900)),
            "a creep never reached the meet: ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
    }
    assert_eq!(world.victor(), None, "no tower or hero has fallen enough");
}

/// The bug this guards against: the demo lane was drawn through the towers
/// the way the big map's lanes are, and every wave hooked around its own
/// tower instead of walking the road past it.
#[test]
fn the_demo_waves_walk_the_road_and_not_through_their_towers() {
    let map = crate::game::map_of(bota_proto::MapId(1));
    let towers = [
        rules::DEMO_RADIANT_TOWERS[0].2,
        rules::DEMO_DIRE_TOWERS[0].2,
    ];
    let footprint = crate::game::plan_radius(rules::units(rules::TOWER_COLLISION));
    let routes = crate::game::lane_routes(map);
    for side in &routes {
        let route = &side[0];
        assert!(!route.is_empty(), "the lane is walked");
        for waypoint in route {
            for tower in towers {
                assert!(
                    !waypoint.within(tower, footprint),
                    "a waypoint at ({},{}) aims into the tower at ({},{})",
                    waypoint.x.to_int(),
                    waypoint.y.to_int(),
                    tower.x.to_int(),
                    tower.y.to_int()
                );
            }
        }
    }
}

/// The bug this guards against: on lanes whose spawner stands ahead of its
/// own rearmost tower, the route began behind the wave, and every fresh
/// wave walked back to its own tower before turning around.
///
/// Going round a tower's footprint gives back a little of the way along
/// the lane; walking back further than that footprint is walking back.
#[test]
fn no_route_on_any_map_walks_a_wave_backwards() {
    let slack = i64::from(
        crate::game::plan_radius(rules::units(rules::TOWER_COLLISION)).to_int()
            + rules::WIDEST_MARCHER,
    );
    for map_id in [bota_proto::MapId(0), bota_proto::MapId(1)] {
        let map = crate::game::map_of(map_id);
        let routes = crate::game::lane_routes(map);
        for (ti, team) in [bota_proto::Team::Radiant, bota_proto::Team::Dire]
            .into_iter()
            .enumerate()
        {
            for lane in map.lanes() {
                let route = &routes[ti][usize::from(lane)];
                assert!(!route.is_empty(), "the lane is walked");
                let mut line = crate::game::lane_polyline(map, lane);
                if team == bota_proto::Team::Dire {
                    line.reverse();
                }
                let mut furthest =
                    lane_progress(&line, crate::game::creep_spawn_pos(map, team, lane));
                for (i, w) in route.iter().enumerate() {
                    let here = lane_progress(&line, *w);
                    assert!(
                        here + slack >= furthest,
                        "{team:?} lane {lane} on map {map_id:?} walks back along the lane at waypoint {i} ({},{}): {here} after {furthest}",
                        w.x.to_int(),
                        w.y.to_int()
                    );
                    furthest = furthest.max(here);
                }
            }
        }
    }
}
