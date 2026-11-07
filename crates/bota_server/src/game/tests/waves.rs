//! Lane waves, their routes, and the structures that open lanes.

use bota_core::{creep_spawn_pos, isqrt64, lane_polyline, map_of, rules};

use crate::game::{Entity, Lane, UnitOrder, World, lane_routes, plan_radius};
use bota_proto::{CreepTier, Fixed, MapId, Team, UnitKind, Vec2};

use super::support::*;

/// Runs a match past its first waves and half a minute on: how many came
/// out, and each still standing with its spawner's spot and its own.
fn the_first_waves_half_a_minute_on(world: &mut World) -> (usize, Vec<(Entity, Vec2, Vec2)>) {
    for _ in 0..=world.schedule().first_wave() {
        world.advance_trusted(&[]);
    }
    let first: Vec<(Entity, Vec2)> = world
        .entities
        .iter()
        .filter(|e| world.march.get(*e).is_some())
        .map(|e| {
            let team = world.team.get(e).copied().expect("has a side");
            let lane = world.lane.get(e).copied().expect("has a lane");
            (e, creep_spawn_pos(world.map, team, lane.0))
        })
        .collect();
    for _ in 0..(30 * rules::TICKS_PER_SECOND) {
        world.advance_trusted(&[]);
    }
    let standing = first
        .iter()
        .filter(|(creep, _)| world.entities.contains(*creep))
        .map(|&(creep, spawn)| {
            (
                creep,
                spawn,
                world.transform.get(creep).expect("standing").pos,
            )
        })
        .collect();
    (first.len(), standing)
}

#[test]
fn a_wave_carries_one_flag_from_the_fifth_wave_on() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let fifth = map.schedule().first_wave() + 4 * rules::WAVE_PERIOD_TICKS;
    while world.tick < fifth {
        world.step();
    }
    let flags = world
        .entities
        .iter()
        .filter(|e| world.kind.get(*e) == Some(&UnitKind::CreepFlagbearer))
        .count();
    assert_eq!(flags, 2 * usize::from(map.lanes), "one a lane a side");
}

#[test]
fn a_creep_off_its_route_rejoins_it_ahead_and_never_walks_back() {
    let mut world = the_demo_lane_at_its_first_wave();
    let creep = *radiant_marchers(&world).first().expect("a wave came out");
    let route = world.walked_lanes()[0][0].clone();
    assert!(route.len() >= 3, "the lane has corners to be ahead of");
    // Carried past its next two waypoints and off to one side of the road.
    let ahead = route[2] + Vec2::from_ints(0, 300);
    if let Some(at) = world.transform.get_mut(creep) {
        at.pos = ahead;
    }
    let was = lane_progress(&route, ahead);
    world.step();
    let Some(UnitOrder::AttackMove { pos }) = world.orders.get(creep).map(|o| o.current) else {
        panic!("it is sent somewhere");
    };
    assert!(
        lane_progress(&route, pos) + 40 >= was,
        "it is sent on, not back: to {} from {}",
        lane_progress(&route, pos),
        was
    );
    run(&mut world, 90);
    let now = world.transform.get(creep).expect("alive").pos;
    assert!(
        lane_progress(&route, now) > was,
        "and it gets further along the lane"
    );
}

#[test]
fn a_creep_is_sent_one_way_at_a_time() {
    let mut world = the_demo_lane_at_its_first_wave();
    let creep = *radiant_marchers(&world).first().expect("a wave came out");
    // Pushed off the route: the mark it is given must not change from tick
    // to tick while nothing about it changes.
    if let Some(at) = world.transform.get_mut(creep) {
        at.pos += Vec2::from_ints(-200, 200);
    }
    world.step();
    let first = world.orders.get(creep).map(|o| o.current);
    world.step();
    let second = world.orders.get(creep).map(|o| o.current);
    assert_eq!(first, second, "it is not pulled two ways in one breath");
    let was = world.transform.get(creep).expect("alive").pos;
    run(&mut world, 60);
    let later = world.transform.get(creep).expect("alive").pos;
    assert!(
        !later.within(was, rules::units(40)),
        "and it actually goes somewhere: {was:?} then {later:?}"
    );
}

#[test]
fn no_creep_of_the_first_waves_is_left_wrestling_its_own_base() {
    let mut world = a_match_with_nobody_in_it(7, MapId(0));
    let (spawned, standing) = the_first_waves_half_a_minute_on(&mut world);
    assert_eq!(spawned, 24, "four creeps a lane, three lanes, two sides");
    // What died, died out on the lane; the mid waves met in the middle of
    // no-man's land, four against four.
    let meet = Vec2::from_ints(8706, 8838);
    for &(creep, spawn, at) in &standing {
        assert!(
            !at.within(spawn, rules::units(1500)),
            "a creep is still beside its spawner at ({},{})",
            at.x.to_int(),
            at.y.to_int()
        );
        if world.lane.get(creep).copied() == Some(Lane(rules::LANE_MID)) {
            assert!(
                at.within(meet, rules::units(900)),
                "a mid creep never reached the river: ({},{})",
                at.x.to_int(),
                at.y.to_int()
            );
        }
    }
}

#[test]
fn a_wave_walks_over_where_its_tower_stood_once_it_has_fallen() {
    let mut world = a_match_with_nobody_in_it(7, MapId(0));
    let (lane, _, tower) = rules::RADIANT_TOWERS[2];
    assert_eq!(lane, rules::LANE_MID, "the Radiant mid tier three");
    let footprint = plan_radius(rules::units(rules::TOWER_COLLISION));
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
    for _ in 0..=world.schedule().first_wave() {
        world.advance_trusted(&[]);
    }
    let mut nearest = i64::MAX;
    for _ in 0..(20 * rules::TICKS_PER_SECOND) {
        world.advance_trusted(&[]);
        for creep in radiant_marchers(&world) {
            if world.lane.get(creep) == Some(&Lane(rules::LANE_MID)) {
                let at = world.transform.get(creep).expect("standing").pos;
                nearest = nearest.min(isqrt64(at.distance_squared(tower)) >> 16);
            }
        }
    }
    assert!(
        nearest < i64::from(rules::TOWER_COLLISION),
        "the wave walks where the tower's body was: nearest {nearest}"
    );
}

#[test]
fn a_lane_opens_tower_by_tower_into_its_barracks() {
    let mut world = World::on_map(map_of(MapId(0)));
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
    let mut world = World::on_map(map_of(MapId(0)));
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
    let mut world = World::on_map(map_of(MapId(0)));
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
    let mut world = a_match_with_nobody_in_it(9, MapId(0));
    // Whole barracks: plain waves.
    let plain = next_mid_wave(&mut world, Team::Dire);
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
    let dire = next_mid_wave(&mut world, Team::Dire);
    let hp_of = |world: &World, e: Entity| world.stats.get(e).expect("settled").max_hp;
    assert!(
        dire.iter().any(
            |e| hp_of(&world, *e) >= Fixed::from_int(rules::SUPER_MELEE_HP)
                && world.kind.get(*e) == Some(&UnitKind::CreepMelee)
        ),
        "the melee spawn super"
    );
    assert!(
        dire.iter()
            .filter(|e| world.kind.get(**e) == Some(&UnitKind::CreepRanged))
            .all(|e| hp_of(&world, *e) < Fixed::from_int(rules::SUPER_RANGED_HP)),
        "the ranged do not"
    );
    let radiant = next_mid_wave(&mut world, Team::Radiant);
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
    let mega = next_mid_wave(&mut world, Team::Dire);
    let mega_melee = mega
        .iter()
        .find(|e| world.kind.get(**e) == Some(&UnitKind::CreepMelee))
        .expect("a melee creep spawned");
    let stats = world.stats.get(*mega_melee).expect("settled");
    assert_eq!(
        (stats.max_hp, stats.damage, stats.attack_time),
        (Fixed::from_int(1270), 101, rules::CREEP_ATTACK_TIME),
        "a mega melee is a fully grown super one, swinging at the plain pace"
    );
}

#[test]
fn the_demo_waves_march_out_and_meet_between_the_towers() {
    let mut world = a_match_with_nobody_in_it(11, MapId(1));
    let (spawned, standing) = the_first_waves_half_a_minute_on(&mut world);
    assert_eq!(spawned, 8, "one wave a side on the one lane");
    // The survivors grind in the middle of the lane, and with no Ancient
    // standing there is nothing to win by.
    let meet = Vec2::from_ints(8850, 9020);
    for (_, spawn, at) in standing {
        assert!(
            !at.within(spawn, rules::units(600)),
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

/// No waypoint of the demo lanes falls inside a tower's footprint.
#[test]
fn the_demo_waves_walk_the_road_and_not_through_their_towers() {
    let map = map_of(MapId(1));
    let towers = [
        rules::DEMO_RADIANT_TOWERS[0].2,
        rules::DEMO_DIRE_TOWERS[0].2,
    ];
    let footprint = plan_radius(rules::units(rules::TOWER_COLLISION));
    let routes = lane_routes(map);
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

/// Giving back up to a tower's footprint plus the widest marcher counts as
/// going round a tower, not walking back.
#[test]
fn no_route_on_any_map_walks_a_wave_backwards() {
    let slack = i64::from(
        plan_radius(rules::units(rules::TOWER_COLLISION)).to_int() + rules::WIDEST_MARCHER,
    );
    for map_id in [MapId(0), MapId(1)] {
        let map = map_of(map_id);
        let routes = lane_routes(map);
        for (ti, team) in [Team::Radiant, Team::Dire].into_iter().enumerate() {
            for lane in map.lanes() {
                let route = &routes[ti][usize::from(lane)];
                assert!(!route.is_empty(), "the lane is walked");
                let mut line = lane_polyline(map, lane);
                if team == Team::Dire {
                    line.reverse();
                }
                let mut furthest = lane_progress(&line, creep_spawn_pos(map, team, lane));
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

#[test]
fn upgraded_creeps_use_735_damage_and_siege_needs_only_ranged_barracks() {
    let mut world = a_match_with_nobody_in_it(321, MapId(0));
    fell_at(&mut world, rules::RADIANT_BARRACKS[1].2);
    world.spawn_wave(rules::FIRST_SIEGE_WAVE);
    let find = |kind| {
        world
            .entities
            .iter()
            .find(|e| {
                world.team.get(*e) == Some(&Team::Dire)
                    && world.kind.get(*e) == Some(&kind)
                    && world.lane.get(*e) == Some(&Lane(rules::LANE_MID))
            })
            .expect("Dire mid creep")
    };
    assert_eq!(
        world
            .stats
            .get(find(UnitKind::CreepSiege))
            .expect("settled")
            .damage,
        51,
        "the ranged barracks alone upgrades the next siege wave"
    );
    assert_eq!(
        world
            .stats
            .get(find(UnitKind::CreepRanged))
            .expect("settled")
            .damage,
        46,
        "7.35 added five damage to super and mega creeps"
    );
}

#[test]
fn each_barracks_upgrades_only_its_lane_and_kind_and_all_six_upgrade_every_lane() {
    for side in [Team::Radiant, Team::Dire] {
        for ranged in [false, true] {
            let mut world = a_match_with_nobody_in_it(321, MapId(0));
            let enemy = if side == Team::Radiant {
                Team::Dire
            } else {
                Team::Radiant
            };
            let barracks = world.map.barracks[bota_core::team_index(side)];
            let at = barracks
                .iter()
                .find(|b| b.0 == rules::LANE_MID && b.1 == ranged)
                .expect("mid barracks")
                .2;
            fell_at(&mut world, at);
            world.spawn_wave(rules::FIRST_SIEGE_WAVE);
            for entity in world
                .entities
                .iter()
                .filter(|e| world.march.get(*e).is_some())
            {
                let kind = *world.kind.get(entity).expect("kind");
                let lane = world.lane.get(entity).expect("lane").0;
                let belongs = world.team.get(entity) == Some(&enemy) && lane == rules::LANE_MID;
                let upgrades = if ranged {
                    matches!(kind, UnitKind::CreepRanged | UnitKind::CreepSiege)
                } else {
                    kind == UnitKind::CreepMelee
                };
                let expected = if belongs && upgrades {
                    CreepTier::Super
                } else {
                    CreepTier::Normal
                };
                assert_eq!(
                    shown(&world, None, entity).creep_tier,
                    Some(expected),
                    "{side:?} ranged={ranged} lane={lane} {kind:?}"
                );
            }
            for &(_, _, at) in barracks {
                if world.entities.iter().any(|e| {
                    world.kind.get(e) == Some(&UnitKind::Barracks) && world.pos(e) == Some(at)
                }) {
                    fell_at(&mut world, at);
                }
            }
            let before: Vec<_> = world.entities.iter().collect();
            world.spawn_wave(rules::FIRST_SIEGE_WAVE);
            let mut lanes = std::collections::BTreeSet::new();
            for entity in world
                .entities
                .iter()
                .filter(|e| !before.contains(e) && world.march.get(*e).is_some())
            {
                let kind = *world.kind.get(entity).expect("kind");
                let mega =
                    world.team.get(entity) == Some(&enemy) && kind != UnitKind::CreepFlagbearer;
                let expected = if mega {
                    CreepTier::Mega
                } else {
                    CreepTier::Normal
                };
                assert_eq!(shown(&world, None, entity).creep_tier, Some(expected));
                if mega {
                    lanes.insert(world.lane.get(entity).expect("lane").0);
                }
            }
            assert_eq!(lanes.len(), 3, "mega creeps spawn in every lane");
        }
    }
}

#[test]
fn mega_creep_bounties_keep_growing_past_the_combat_upgrade_cap() {
    let mut world = a_match_with_nobody_in_it(322, MapId(0));
    for &(_, _, at) in &rules::RADIANT_BARRACKS {
        fell_at(&mut world, at);
    }
    world.spawn_wave(1 + rules::WAVE_UPGRADE_PERIOD * (rules::WAVE_UPGRADE_CAP + 1));
    let ranged = world
        .entities
        .iter()
        .find(|e| {
            world.team.get(*e) == Some(&Team::Dire)
                && world.kind.get(*e) == Some(&UnitKind::CreepRanged)
        })
        .expect("mega ranged");
    assert_eq!(
        world.stats.get(ranged).expect("stats").max_hp,
        Fixed::from_int(1015)
    );
    let bounty = world.bounty.get(ranged).expect("bounty");
    assert_eq!(
        (bounty.gold.min, bounty.gold.max, bounty.xp),
        (112, 118, 22),
        "31 upgrades affect gold; mega combat stats and experience stay fixed"
    );
}

#[test]
fn the_last_barracks_announce_mega_creeps_once_when_their_deaths_settle_together() {
    use bota_proto::EventKind;
    for together in [true, false] {
        let mut world = a_match_with_nobody_in_it(323, MapId(0));
        for &(_, _, at) in rules::RADIANT_BARRACKS.iter().take(4) {
            fell_at(&mut world, at);
        }
        let last: Vec<_> = rules::RADIANT_BARRACKS
            .iter()
            .skip(4)
            .map(|b| structure_at(&world, b.2))
            .collect();
        // Damage has already reached zero for both; burial may arrive as one batch or two.
        for &entity in &last {
            world.health.get_mut(entity).expect("barracks health").hp = Fixed::ZERO;
        }
        let mut events = Vec::new();
        if together {
            world.bury(
                last.iter().map(|entity| (*entity, None)).collect(),
                &mut events,
            );
        } else {
            for &entity in &last {
                world.bury(vec![(entity, None)], &mut events);
            }
        }
        let announcements: Vec<_> = events
            .iter()
            .filter_map(|event| {
                if let EventKind::MegaCreeps { team } = event.kind() {
                    Some(team)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(announcements, vec![Team::Dire], "together={together}");
        for side in [Some(Team::Radiant), Some(Team::Dire), None] {
            assert_eq!(
                crate::game_loop::visible_events(&events, side)
                    .iter()
                    .filter(|event| matches!(event, EventKind::MegaCreeps { team: Team::Dire }))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn a_creeps_spawn_tier_changes_its_world_fingerprint() {
    for (super_def, mega_def) in [
        (&bota_core::SUPER_MELEE_CREEP, &bota_core::MEGA_MELEE_CREEP),
        (&bota_core::SUPER_SIEGE_CREEP, &bota_core::MEGA_SIEGE_CREEP),
    ] {
        let spawn = |def| {
            let mut world = World::new();
            let creep = world.spawn_creep(def, Team::Radiant, Vec2::from_ints(5000, 5000), 0, 0);
            world.settle();
            world.health.get_mut(creep).expect("creep health").hp = Fixed::from_int(200);
            (world, creep)
        };
        let (super_world, super_creep) = spawn(super_def);
        let (mega_world, mega_creep) = spawn(mega_def);
        assert_eq!(
            shown(&super_world, None, super_creep).creep_tier,
            Some(CreepTier::Super)
        );
        assert_eq!(
            shown(&mega_world, None, mega_creep).creep_tier,
            Some(CreepTier::Mega)
        );
        assert_ne!(
            super_world.hash(),
            mega_world.hash(),
            "{:?} retains its historical spawn tier",
            super_def.kind
        );
    }
}

#[test]
fn normal_creeps_use_no_tier_hash_row_even_after_an_upgraded_slot_is_reused() {
    let mut world = World::new();
    let upgraded = world.spawn_creep(
        &bota_core::MEGA_MELEE_CREEP,
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        0,
        0,
    );
    assert_eq!(world.creep_tier.get(upgraded), Some(&CreepTier::Mega));
    assert!(world.despawn(upgraded));
    let normal = world.spawn_creep(
        &bota_core::MELEE_CREEP,
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        0,
        0,
    );
    world.settle();
    assert_eq!(
        normal.index(),
        upgraded.index(),
        "the retired slot is reused with a fresh generation"
    );
    assert_eq!(world.creep_tier.get(normal), None);
    assert_eq!(
        shown(&world, None, normal).creep_tier,
        Some(CreepTier::Normal)
    );
}

#[test]
fn a_barracks_denied_by_its_own_side_still_upgrades_the_enemy_lane() {
    for ranged in [false, true] {
        let mut world = a_match_with_nobody_in_it(324, MapId(0));
        let at = rules::RADIANT_BARRACKS
            .iter()
            .find(|b| b.0 == rules::LANE_MID && b.1 == ranged)
            .expect("mid barracks")
            .2;
        let barracks = structure_at(&world, at);
        let killer = world.spawn_creep(
            &bota_core::MELEE_CREEP,
            Team::Radiant,
            Vec2::from_ints(5000, 5000),
            0,
            0,
        );
        world.health.get_mut(barracks).expect("barracks health").hp = Fixed::ZERO;
        let mut events = Vec::new();
        world.bury(vec![(barracks, Some(killer))], &mut events);
        assert!(events.iter().any(|event| matches!(
            event.kind(),
            bota_proto::EventKind::Died { denied: true, .. }
        )));
        world.spawn_wave(rules::FIRST_SIEGE_WAVE);
        let kind = if ranged {
            UnitKind::CreepRanged
        } else {
            UnitKind::CreepMelee
        };
        let creep = world
            .entities
            .iter()
            .find(|e| {
                world.team.get(*e) == Some(&Team::Dire)
                    && world.kind.get(*e) == Some(&kind)
                    && world.lane.get(*e) == Some(&Lane(rules::LANE_MID))
            })
            .expect("the opposing lane spawned");
        assert_eq!(
            shown(&world, None, creep).creep_tier,
            Some(CreepTier::Super)
        );
    }
}
