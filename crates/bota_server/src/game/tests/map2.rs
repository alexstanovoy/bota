//! Map2 geometry, waves, and end-of-tick match completion.

use bota_core::{
    CellGrid, MAP2_DEATH_LIMIT, MAP2_GAME_TICKS, MAP2_ID, MAP2_TICK_CAP, MAPS, hero_spawn_pos,
    item_def, lane_polyline, lane_route, map_of, rules, wave_plan,
};

use crate::game::{
    Clearance, Command, Entity, Event, EventVisibility, MatchConfig, Seat, World, lane_routes,
    terrain_of, tree_positions, wire_id,
};
use bota_proto::{
    DamageKind, EventKind, Fixed, HeroId, ItemId, MapId, Order, Pick, ReplayRecord, ServerMsg,
    SlotId, Team, TickMode, UnitKind, Vec2, decode_payload, encode_frame_to_vec,
};

use super::support::advance_validated;

const MID_MAP: MapId = MapId(2);
const TICK_CAP: u32 = 27_900;
const FINAL_WAVE: u32 = (TICK_CAP - rules::FIRST_WAVE_TICK) / rules::WAVE_PERIOD_TICKS + 1;

fn config(map: MapId) -> MatchConfig {
    MatchConfig {
        match_id: 23,
        master_key: [17; 32],
        picks: vec![
            Pick {
                slot: SlotId(0),
                team: Team::Radiant,
                hero: HeroId::SHADOW_FIEND,
            },
            Pick {
                slot: SlotId(1),
                team: Team::Dire,
                hero: HeroId::SHADOW_FIEND,
            },
        ],
        map,
        tick_rate: 30,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 150,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
    }
}

fn match_world(map: MapId) -> World {
    let config = config(map);
    let world = World::for_match(&config, config.rng());
    assert_eq!(world.seats.len(), 2);
    assert_eq!(world.victor(), None);
    world
}

fn tower(world: &World, team: Team, lane: u8) -> Entity {
    world
        .entities
        .iter()
        .find(|&entity| {
            world.kind.get(entity) == Some(&UnitKind::Tower)
                && world.team.get(entity) == Some(&team)
                && world.lane.get(entity).is_some_and(|value| value.0 == lane)
                && world.tier.get(entity).is_some_and(|value| value.0 == 1)
        })
        .expect("a tier-one tower on the requested lane")
}

fn lethal_hit(world: &mut World, victim: Entity) {
    assert!(world.alive(victim));
    assert!(
        !world
            .stats
            .get(victim)
            .expect("settled victim")
            .invulnerable
    );
    world.push_hit(None, victim, 30_000, DamageKind::Pure);
}

/// Lands a lethal hit on a side's tier-one mid tower and hands it back.
fn topple(world: &mut World, team: Team) -> Entity {
    let victim = tower(world, team, rules::LANE_MID);
    lethal_hit(world, victim);
    victim
}

/// Seat 0 buying the first item of the catalog.
fn late_buy() -> Command {
    Command {
        slot: SlotId(0),
        unit: None,
        order: Order::Buy {
            item: ItemId::BOOTS,
        },
    }
}

fn assert_destroyed(events: &[Event], victim: Entity, team: Team) {
    let destroyed = events
        .iter()
        .filter(|event| {
            event.kind
                == EventKind::StructureDestroyed {
                    unit: wire_id(victim),
                    team,
                }
        })
        .collect::<Vec<_>>();
    assert_eq!(destroyed.len(), 1);
    assert_eq!(destroyed[0].visible_to, EventVisibility::Everyone);
}

#[test]
fn map0_and_map2_share_every_landmark_lane_route_and_tree() {
    let full = map_of(MapId(0));
    let mid = map_of(MID_MAP);

    assert_eq!(mid.fountains, full.fountains);
    assert_eq!(mid.ancients, full.ancients);
    assert_eq!(mid.radiant_towers, full.radiant_towers);
    assert_eq!(mid.dire_towers, full.dire_towers);
    assert_eq!(mid.barracks, full.barracks);
    assert_eq!(mid.protection, full.protection);
    assert_eq!(mid.creep_spawns, full.creep_spawns);
    assert_eq!(mid.lane_corners, full.lane_corners);
    assert_eq!(mid.lane_through_towers, full.lane_through_towers);
    assert_eq!(mid.camps, full.camps);
    let (mid_ground, full_ground) = (terrain_of(mid), terrain_of(full));
    assert_eq!(mid_ground.trees, full_ground.trees);
    assert_eq!(mid_ground.lane_clear, full_ground.lane_clear);
    assert_eq!(mid_ground.fow_blockers, full_ground.fow_blockers);
    assert_eq!(mid_ground.terrain_rle, full_ground.terrain_rle);
    assert_eq!(mid.lanes().collect::<Vec<_>>(), vec![0, 1, 2]);
    for lane in [rules::LANE_MID, rules::LANE_TOP, rules::LANE_BOT] {
        assert_eq!(lane_polyline(mid, lane), lane_polyline(full, lane));
        for team in [Team::Radiant, Team::Dire] {
            assert_eq!(lane_route(mid, team, lane), lane_route(full, team, lane));
        }
    }
    assert_eq!(lane_routes(mid), lane_routes(full));
    assert!(!tree_positions(full, crate::game::terrain_of(full)).is_empty());
    assert_eq!(
        tree_positions(mid, crate::game::terrain_of(mid)),
        tree_positions(full, crate::game::terrain_of(full))
    );
}

#[test]
fn map0_and_map2_passability_terrain_and_sight_match_in_every_cell() {
    let full = World::on_map(map_of(MapId(0)));
    let mid = World::on_map(map_of(MID_MAP));
    let full_field = Clearance::of_map(full.map);
    let mid_field = Clearance::of_map(mid.map);
    assert_eq!(full_field.circles(), mid_field.circles());
    assert_eq!(mid.sight_block, full.sight_block);

    for y in 0..rules::GRID_CELLS {
        for x in 0..rules::GRID_CELLS {
            let at = CellGrid::cell_center((x, y));
            assert_eq!(
                mid_field.terrain().cell_open(x, y),
                full_field.terrain().cell_open(x, y)
            );
            assert_eq!(
                mid.clearance.terrain().cell_open(x, y),
                full.clearance.terrain().cell_open(x, y)
            );
            assert_eq!(
                mid.ground.cell_walkable(x, y),
                full.ground.cell_walkable(x, y)
            );
            assert_eq!(mid.ground.tier(at), full.ground.tier(at));
            assert_eq!(mid.ground.water(at), full.ground.water(at));
        }
    }
}

#[test]
fn map0_and_map2_initial_worlds_have_identical_buildings_heroes_and_shops() {
    let full = match_world(MapId(0));
    let mid = match_world(MID_MAP);

    assert_eq!(mid.view_full(), full.view_full());
    for team in [Team::Radiant, Team::Dire] {
        assert_eq!(mid.view(team), full.view(team));
        assert_eq!(
            hero_spawn_pos(mid.map, team),
            hero_spawn_pos(full.map, team)
        );
    }
    let full_info = config(MapId(0)).info();
    let mid_info = config(MID_MAP).info();
    assert_eq!(mid_info.trees, full_info.trees);
    assert_eq!(mid_info.terrain_rle, full_info.terrain_rle);
    assert_eq!(mid_info.opaque_cells, full_info.opaque_cells);
    assert_eq!(mid_info.shop, full_info.shop);
}

#[test]
fn map2_spawns_only_mid_waves_for_both_sides_through_the_cap() {
    let map = map_of(MID_MAP);
    for wave in 1..=FINAL_WAVE {
        let mut world = World::on_map(map);
        world.tick = rules::FIRST_WAVE_TICK + (wave - 1) * rules::WAVE_PERIOD_TICKS;
        let plan = wave_plan(wave);
        let per_side = (plan.melee + plan.ranged + plan.siege) as usize;

        world.spawn_waves();

        for team in [Team::Radiant, Team::Dire] {
            let creeps = world
                .entities
                .iter()
                .filter(|&entity| {
                    world.march.get(entity).is_some() && world.team.get(entity) == Some(&team)
                })
                .collect::<Vec<_>>();
            assert_eq!(creeps.len(), per_side, "wave {wave}, {team:?}");
            for creep in creeps {
                assert_eq!(
                    world.lane.get(creep).map(|lane| lane.0),
                    Some(rules::LANE_MID)
                );
            }
        }
    }
}

#[test]
fn map2_pregame_and_between_wave_ticks_spawn_no_lane_creeps() {
    for tick in [0, rules::FIRST_WAVE_TICK - 1, rules::FIRST_WAVE_TICK + 1] {
        let mut world = World::on_map(map_of(MID_MAP));
        world.tick = tick;

        world.spawn_waves();

        assert!(
            !world
                .entities
                .iter()
                .any(|entity| world.march.get(entity).is_some())
        );
    }
}

#[test]
fn map2_first_hero_death_allows_respawn_and_second_death_loses() {
    for (seat, winner) in [(0, Team::Dire), (1, Team::Radiant)] {
        let mut world = match_world(MID_MAP);
        let first = world.seats[seat].unit.expect("first life");
        lethal_hit(&mut world, first);

        world.advance(&[]);

        assert_eq!(world.seats[seat].deaths, 1);
        assert_eq!(world.victor(), None);
        assert!(world.seats[seat].unit.is_none());
        world.seats[seat].respawn_left = 1;
        world.advance(&[]);
        let second = world.seats[seat].unit.expect("second life respawned");
        assert_ne!(second, first);
        lethal_hit(&mut world, second);
        world.advance(&[]);
        assert_eq!(world.seats[seat].deaths, 2);
        assert_eq!(world.victor(), Some(winner));
    }
}

#[test]
fn map2_hero_lives_are_counted_per_side_across_seats() {
    let mut world = match_world(MID_MAP);
    let slot = SlotId(2);
    let mut teammate = Seat::new(
        slot,
        Team::Radiant,
        HeroId::SHADOW_FIEND,
        0,
        rules::STASH_SLOTS,
    );
    teammate.unit = Some(world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        slot,
        HeroId::SHADOW_FIEND,
    ));
    teammate.deaths = 1;
    world.seats.push(teammate);
    let victim = world.seats[0].unit.expect("Radiant hero");
    lethal_hit(&mut world, victim);

    world.advance(&[]);

    assert_eq!(world.seats[0].deaths, 1);
    assert_eq!(world.seats[2].deaths, 1);
    assert_eq!(world.victor(), Some(Team::Dire));
}

#[test]
fn map2_first_tower_loss_on_any_lane_ends_with_public_structure_reason() {
    for (loser, winner) in [(Team::Radiant, Team::Dire), (Team::Dire, Team::Radiant)] {
        for lane in [rules::LANE_MID, rules::LANE_TOP, rules::LANE_BOT] {
            let mut world = match_world(MID_MAP);
            let victim = tower(&world, loser, lane);
            lethal_hit(&mut world, victim);

            let events = world.advance(&[]);

            assert_eq!(world.victor(), Some(winner));
            assert_destroyed(&events, victim, loser);
            assert_eq!(world.seats[0].deaths, 0);
            assert_eq!(world.seats[1].deaths, 0);
        }
    }
}

#[test]
fn map2_immortal_structures_survive_sustained_attack_and_the_match_goes_on() {
    let mut cfg = config(MID_MAP);
    cfg.immortal_structures = true;
    let mut world = World::for_match(&cfg, cfg.rng());
    let victim = tower(&world, Team::Dire, rules::LANE_MID);
    let mut felt = 0;
    for _ in 0..30 {
        world.push_hit(None, victim, 30_000, DamageKind::Pure);
        let events = world.advance(&[]);
        felt += events
            .iter()
            .filter(|event| matches!(&event.kind, EventKind::Damaged { target, amount, .. } if *target == wire_id(victim) && *amount > 0))
            .count();
        assert!(world.alive(victim));
        assert!(world.health.get(victim).expect("health").hp >= Fixed::from_int(1));
        assert_eq!(world.victor(), None);
    }
    assert!(felt > 0, "the tower still takes and reports damage");
}

#[test]
fn map2_immortal_heroes_survive_lethal_blows_and_no_death_ends_the_match() {
    let mut cfg = config(MID_MAP);
    cfg.immortal_heroes = true;
    let mut world = World::for_match(&cfg, cfg.rng());
    for _ in 0..3 {
        for seat in 0..2 {
            let hero = world.seats[seat].unit.expect("hero");
            lethal_hit(&mut world, hero);
        }
        world.advance(&[]);
        for seat in 0..2 {
            let hero = world.seats[seat].unit.expect("hero");
            assert!(world.alive(hero));
            let hp = world.health.get(hero).expect("health").hp;
            assert!(
                hp >= Fixed::from_int(1) && hp < Fixed::from_int(2),
                "{hp:?}"
            );
            assert_eq!(world.seats[seat].deaths, 0);
        }
        assert_eq!(world.victor(), None);
    }
}

#[test]
fn map2_courier_death_does_not_spend_a_hero_life_or_end_the_match() {
    let mut world = match_world(MID_MAP);
    let courier = world.seats[0].courier.expect("Radiant courier");
    lethal_hit(&mut world, courier);

    world.advance(&[]);

    assert!(!world.alive(courier));
    assert_eq!(world.seats[0].deaths, 0);
    assert_eq!(world.victor(), None);
}

#[test]
fn map2_both_sides_losing_on_one_tick_draw_in_either_hit_order() {
    // Per side, Radiant first: a tower loss, or else a second hero death.
    for by_tower in [[false, false], [true, true], [true, false], [false, true]] {
        for reverse in [false, true] {
            let case = format!("towers {by_tower:?}, reversed {reverse}");
            let mut world = match_world(MID_MAP);
            let mut victims = Vec::new();
            for (seat, team) in [(0, Team::Radiant), (1, Team::Dire)] {
                if by_tower[seat] {
                    victims.push(tower(&world, team, rules::LANE_MID));
                } else {
                    world.seats[seat].deaths = 1;
                    victims.push(world.seats[seat].unit.expect("second life"));
                }
            }
            if reverse {
                victims.reverse();
            }
            for victim in victims {
                lethal_hit(&mut world, victim);
            }

            let events = world.advance(&[]);

            assert_eq!(world.victor(), Some(Team::Neutral), "{case}");
            let felled = events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::StructureDestroyed { .. }))
                .count();
            assert_eq!(felled, by_tower.iter().filter(|&&t| t).count(), "{case}");
            for (seat, tower_lost) in by_tower.into_iter().enumerate() {
                let deaths = if tower_lost { 0 } else { 2 };
                assert_eq!(world.seats[seat].deaths, deaths, "{case}: seat {seat}");
            }
        }
    }
}

#[test]
fn map2_simultaneous_first_hero_deaths_do_not_end_the_match() {
    let mut world = match_world(MID_MAP);
    for seat in 0..2 {
        let victim = world.seats[seat].unit.expect("first life");
        lethal_hit(&mut world, victim);
    }

    world.advance(&[]);

    assert_eq!(world.victor(), None);
    assert_eq!(world.seats[0].deaths, 1);
    assert_eq!(world.seats[1].deaths, 1);
}

#[test]
fn map2_cap_draws_at_fifteen_game_minutes_plus_unchanged_pregame() {
    let mut world = match_world(MID_MAP);
    world.tick = TICK_CAP - 2;

    world.advance(&[]);

    assert_eq!(world.tick, TICK_CAP - 1);
    assert_eq!(world.victor(), None);
    world.advance(&[]);
    assert_eq!(
        world.tick,
        rules::PREGAME_TICKS + 15 * 60 * rules::TICKS_PER_SECOND
    );
    assert_eq!(world.victor(), Some(Team::Neutral));
    assert_eq!(world.match_stats().duration, TICK_CAP);
}

#[test]
fn map2_plays_on_past_the_ten_minute_mark() {
    let mut world = match_world(MID_MAP);
    world.tick = 18_899;

    world.advance(&[]);
    world.advance(&[]);

    assert_eq!(world.tick, 18_901);
    assert_eq!(world.victor(), None);
}

#[test]
fn map2_public_limits_and_wave_selection_match_the_match_contract() {
    assert_eq!(MAP2_ID, MID_MAP);
    assert_eq!(map_of(MID_MAP).id, MID_MAP, "map 2 is its own map");
    assert_eq!(map_of(MID_MAP).index(), 2);
    assert_eq!(MAP2_DEATH_LIMIT, 2);
    assert_eq!(MAP2_GAME_TICKS, 27_000);
    assert_eq!(MAP2_TICK_CAP, TICK_CAP);
    assert_eq!(map_of(MID_MAP).wave_lanes, &[rules::LANE_MID]);
    for map in &MAPS {
        assert!(map.lanes <= 3);
        assert!(map.wave_lanes.len() <= usize::from(map.lanes));
        for (index, &lane) in map.wave_lanes.iter().enumerate() {
            assert!(lane < map.lanes);
            assert!(!map.wave_lanes[..index].contains(&lane));
        }
    }
}

#[test]
fn map2_cap_tick_completes_orders_gold_waves_and_tower_death_before_drawing() {
    let mut world = match_world(MID_MAP);
    world.tick = TICK_CAP - 1;
    let hero = world.seats[0].unit.expect("Radiant buyer");
    let boots = ItemId::BOOTS;
    let cost = item_def(boots).expect("boots in catalog").cost;
    let gold = world.seats[0].gold;
    let victim = topple(&mut world, Team::Dire);

    let events = advance_validated(&mut world, None, Order::Buy { item: boots });

    assert!(
        world
            .inventory
            .get(hero)
            .expect("hero's bag")
            .held()
            .any(|stack| stack.id == boots)
    );
    // A second of passive gold, and the fallen tower's team bounty.
    assert_eq!(
        world.seats[0].gold,
        gold - cost + 1 + rules::TOWER_TIER_TEAM_BOUNTY[0]
    );
    let plan = wave_plan(FINAL_WAVE);
    let creeps = world
        .entities
        .iter()
        .filter(|&entity| world.march.get(entity).is_some())
        .count();
    assert_eq!(creeps, 2 * (plan.melee + plan.ranged + plan.siege) as usize);
    assert_destroyed(&events, victim, Team::Dire);
    assert_eq!(world.victor(), Some(Team::Neutral));
    assert_eq!(world.match_stats().duration, TICK_CAP);
}

#[test]
fn map2_already_at_cap_draws_without_running_a_late_step_or_command() {
    for with_command in [false, true] {
        let mut world = match_world(MID_MAP);
        world.tick = TICK_CAP;
        let before = world.view_full();

        let events = if with_command {
            world.advance(&[late_buy()])
        } else {
            world.step()
        };

        assert_eq!(world.tick, TICK_CAP, "command: {with_command}");
        assert!(events.is_empty(), "command: {with_command}");
        assert_eq!(world.view_full(), before, "command: {with_command}");
        assert_eq!(world.match_stats().duration, TICK_CAP);
        assert_eq!(world.victor(), Some(Team::Neutral));
    }
}

#[test]
fn map2_tower_loss_one_tick_before_cap_wins_but_on_cap_draws() {
    for (tick, expected) in [(TICK_CAP - 2, Team::Dire), (TICK_CAP - 1, Team::Neutral)] {
        let mut world = match_world(MID_MAP);
        world.tick = tick;
        let victim = topple(&mut world, Team::Radiant);

        let events = world.advance(&[]);

        assert_eq!(world.victor(), Some(expected));
        assert_eq!(world.match_stats().duration, tick + 1);
        assert_destroyed(&events, victim, Team::Radiant);
    }
}

#[test]
fn map2_second_hero_death_on_cap_is_a_draw_after_death_accounting() {
    let mut world = match_world(MID_MAP);
    world.tick = TICK_CAP - 1;
    world.seats[0].deaths = 1;
    let victim = world.seats[0].unit.expect("second life");
    lethal_hit(&mut world, victim);

    world.advance(&[]);

    assert_eq!(world.seats[0].deaths, 2);
    assert_eq!(world.victor(), Some(Team::Neutral));
}

#[test]
fn map2_terminal_step_and_advance_freeze_world_and_ignore_shop_orders() {
    for cap in [false, true] {
        let mut world = match_world(MID_MAP);
        if cap {
            world.tick = TICK_CAP - 1;
        } else {
            topple(&mut world, Team::Dire);
        }
        world.advance(&[]);
        assert!(world.victor().is_some());
        let before = world.hash();
        let view = world.view_full();
        let stats = world.match_stats();

        let events = world.advance(&[late_buy()]);
        let stepped = world.step();

        assert!(events.is_empty());
        assert!(stepped.is_empty());
        assert_eq!(world.hash(), before);
        assert_eq!(world.view_full(), view);
        assert_eq!(world.match_stats(), stats);
    }
}

#[test]
fn map2_cap_is_independent_of_wall_clock_tick_rate_and_mode() {
    for tick_rate in [1, 30, 120] {
        for mode in [TickMode::Lockstep, TickMode::Realtime] {
            let mut config = config(MID_MAP);
            config.tick_rate = tick_rate;
            config.mode = mode;
            let mut world = World::for_match(&config, config.rng());
            world.tick = TICK_CAP - 1;

            world.advance(&[]);

            assert_eq!(world.victor(), Some(Team::Neutral));
            assert_eq!(world.match_stats().duration, TICK_CAP);
        }
    }
}

#[test]
fn map0_and_map1_keep_their_wave_lanes_and_map1_its_geometry() {
    for (map, count) in [(MapId(0), 3), (MapId(1), 1)] {
        let mut world = World::on_map(map_of(map));
        world.tick = rules::FIRST_WAVE_TICK;
        let plan = wave_plan(1);

        world.spawn_waves();

        assert_eq!(
            world
                .entities
                .iter()
                .filter(|&entity| world.march.get(entity).is_some())
                .count(),
            2 * count * (plan.melee + plan.ranged + plan.siege) as usize
        );
    }
    let demo = map_of(MapId(1));
    assert_eq!(demo.ancients, [None, None]);
    assert_eq!(demo.radiant_towers, &rules::DEMO_RADIANT_TOWERS);
    assert_eq!(demo.dire_towers, &rules::DEMO_DIRE_TOWERS);
    assert_eq!(demo.lanes, 1);
}

#[test]
fn map0_and_map1_do_not_gain_the_map2_cap() {
    for map in [MapId(0), MapId(1)] {
        let mut world = match_world(map);
        world.tick = TICK_CAP - 1;

        world.advance(&[]);
        world.advance(&[]);

        assert_eq!(world.victor(), None);
        assert_eq!(world.tick, TICK_CAP + 1);
    }
}

#[test]
fn map0_and_map1_end_on_neither_a_tower_nor_a_second_hero_death() {
    for map in [MapId(0), MapId(1)] {
        for hero_loss in [false, true] {
            let mut world = match_world(map);
            let victim = if hero_loss {
                world.seats[0].deaths = 1;
                world.seats[0].unit.expect("second life")
            } else {
                tower(&world, Team::Radiant, rules::LANE_MID)
            };
            lethal_hit(&mut world, victim);

            world.advance(&[]);

            assert_eq!(world.victor(), None);
        }
    }
}

#[test]
fn map2_step_and_match_advance_share_completion_events_stats_and_hash() {
    for loser in [Some(Team::Radiant), Some(Team::Dire), None] {
        let mut direct = match_world(MID_MAP);
        let mut adapter = match_world(MID_MAP);
        for world in [&mut direct, &mut adapter] {
            if let Some(team) = loser {
                topple(world, team);
            } else {
                world.tick = TICK_CAP - 1;
            }
        }

        let direct_events = direct.step();
        let adapter_events = adapter.advance(&[]);

        assert!(direct.victor().is_some());
        assert_eq!(direct.victor(), adapter.victor());
        assert_eq!(direct.match_stats(), adapter.match_stats());
        assert_eq!(direct.hash(), adapter.hash());
        assert_eq!(direct.view_full(), adapter.view_full());
        assert_eq!(direct_events.len(), adapter_events.len());
        for (direct, adapter) in direct_events.iter().zip(&adapter_events) {
            assert_eq!(direct.kind, adapter.kind);
            assert_eq!(direct.visible_to, adapter.visible_to);
        }
    }
}

#[test]
fn map2_native_frames_and_replay_preserve_world_result_and_final_statistics() {
    for winner in [Team::Radiant, Team::Dire, Team::Neutral] {
        let mut world = match_world(MID_MAP);
        if winner == Team::Neutral {
            world.tick = TICK_CAP - 1;
        } else {
            let loser = if winner == Team::Radiant {
                Team::Dire
            } else {
                Team::Radiant
            };
            topple(&mut world, loser);
        }
        let events = world.advance(&[]);
        assert_eq!(world.victor(), Some(winner));
        let messages = [
            ServerMsg::MatchStart {
                info: config(MID_MAP).info(),
            },
            ServerMsg::Snapshot {
                view: world.view_full(),
            },
            ServerMsg::Events {
                tick: world.tick,
                events: events.into_iter().map(|event| event.kind).collect(),
            },
            ServerMsg::MatchOver {
                winner: world.victor().expect("completed tick"),
                stats: world.match_stats(),
            },
        ];

        for message in messages {
            let frame = encode_frame_to_vec(&message).expect("native frame encodes");
            let decoded = decode_payload::<ServerMsg>(&frame[4..]).expect("native frame decodes");
            assert_eq!(decoded, message);
            let record = ReplayRecord::Msg(message);
            let frame = encode_frame_to_vec(&record).expect("replay frame encodes");
            assert_eq!(
                decode_payload::<ReplayRecord>(&frame[4..]).expect("replay frame decodes"),
                record
            );
        }
    }
}
