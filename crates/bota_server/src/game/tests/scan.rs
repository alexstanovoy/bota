//! Scan (Dota 7.41f): a side's two shared charges, the first of them 120 seconds past the horn
//! and one more every 210 seconds; a point looked at for eight seconds, once a second, for an
//! enemy hero that stands in 900 units of it, seen or not; the answer, and nothing else, told to
//! the scanning side alone.

use bota_core::{MAP2_ID, ModifierKind, ScanLedger, map_of, rules};
use bota_proto::{Cheat, EventKind, Fixed, Order, RejectReason, SlotId, Team, Vec2};

use super::local::mid_config;
use super::support::{put_on, run};
use crate::game::{Command, Entity, Event, World, perspective::visible_events};

const RADIANT: SlotId = SlotId(0);
const DIRE: SlotId = SlotId(1);

/// Ground inside the Dire base, far from anything of Radiant's, and ground in the Radiant one.
const DIRE_BASE: Vec2 = rules::DIRE_FOUNTAIN_POS;
const RADIANT_BASE: Vec2 = rules::RADIANT_FOUNTAIN_POS;

/// A cheat-enabled match of Shadow Fiend against Shadow Fiend on Solo Mid.
fn duel() -> World {
    let mut cfg = mid_config();
    cfg.cheats = true;
    assert_eq!(cfg.map, MAP2_ID);
    let mut world = World::for_match(&cfg, cfg.rng());
    world.settle();
    world
}

fn hero_of(world: &World, slot: SlotId) -> Entity {
    world
        .seat(slot)
        .and_then(|seat| seat.unit)
        .expect("standing")
}

fn command(slot: SlotId, order: Order) -> Command {
    Command {
        slot,
        unit: None,
        order,
        queue: false,
        seq: 0,
    }
}

/// A seat gives an order that passes validation, and the world advances one tick.
fn give(world: &mut World, slot: SlotId, order: Order) -> Vec<Event> {
    assert_eq!(world.validate_order(slot, None, &order), Ok(()));
    world.advance_trusted(&[command(slot, order)])
}

/// A seat's hero is moved by Dota's `-teleport` to the open ground nearest `to`; where it
/// stands after is told.
fn teleport(world: &mut World, slot: SlotId, to: Vec2) -> Vec2 {
    give(
        world,
        slot,
        Order::Cheat {
            cheat: Cheat::Teleport { at: to },
        },
    );
    world.pos(hero_of(world, slot)).expect("standing")
}

fn run_to(world: &mut World, tick: u32) {
    assert!(world.tick <= tick, "the clock is past {tick}");
    run(world, tick - world.tick);
}

/// The tick a side's first charge comes.
fn first_charge(world: &World) -> u32 {
    world.schedule().scan_first_charge()
}

fn scan(at: Vec2) -> Order {
    Order::Scan { at }
}

/// What the scans of the world have found, by side: the answer of the latest look of each, and
/// the ticks each has left.
fn answers(world: &World) -> Vec<(Team, bool, u32)> {
    world
        .entities
        .iter()
        .filter_map(|entity| {
            let radar = world.radar.get(entity)?;
            // The thinker stands a tick past its duration: what is left of the scan is one less.
            let ticks_left = world.expiry.get(entity)?.ticks_left.saturating_sub(1);
            Some((*world.team.get(entity)?, radar.found, ticks_left))
        })
        .collect()
}

/// The answer of the only scan that looks.
fn answer(world: &World) -> bool {
    let all = answers(world);
    assert_eq!(all.len(), 1, "one scan looks: {all:?}");
    all[0].1
}

fn scan_events(events: &[Event], to: Option<Team>) -> Vec<EventKind> {
    visible_events(events, to)
        .into_iter()
        .filter(|kind| matches!(kind, EventKind::Scan { .. }))
        .collect()
}

/// The Dire hero stands far from anything of Radiant's, and Radiant scans `offset` from its
/// feet at its side's first charge.
fn scanned_enemy(offset: Vec2) -> (World, Vec2) {
    let mut world = duel();
    teleport(&mut world, DIRE, DIRE_BASE);
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    let enemy_at = world.pos(hero_of(&world, DIRE)).expect("standing");
    give(&mut world, RADIANT, scan(enemy_at + offset));
    (world, enemy_at)
}

#[test]
fn a_side_has_no_charge_before_its_first_one_which_comes_120_seconds_past_the_horn() {
    let mut world = duel();
    let ready = first_charge(&world);
    assert_eq!(ready, world.schedule().horn + 120 * rules::TICKS_PER_SECOND);
    for slot in [RADIANT, DIRE] {
        assert_eq!(
            world.validate_order(slot, None, &scan(Vec2::from_ints(8000, 8000))),
            Err(RejectReason::NoScanCharge),
            "no charge in the pregame"
        );
    }
    run_to(&mut world, ready - 1);
    assert_eq!(
        world.validate_order(RADIANT, None, &scan(Vec2::from_ints(8000, 8000))),
        Err(RejectReason::NoScanCharge),
        "none a tick before"
    );
    run_to(&mut world, ready);
    for slot in [RADIANT, DIRE] {
        assert_eq!(
            world.validate_order(slot, None, &scan(Vec2::from_ints(8000, 8000))),
            Ok(()),
            "each side has its own"
        );
    }
}

#[test]
fn the_second_charge_comes_210_seconds_after_the_first_and_then_no_wait_runs() {
    let mut world = duel();
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    let held = world.scan_ledger(0);
    assert_eq!(held.charges, 1);
    assert_eq!(held.next_charge, Some(ready + rules::SCAN_RECHARGE_TICKS));
    run_to(&mut world, ready + rules::SCAN_RECHARGE_TICKS);
    assert_eq!(
        world.scan_ledger(0),
        ScanLedger {
            charges: rules::SCAN_MAX_CHARGES,
            next_charge: None,
        }
    );
    run_to(&mut world, ready + 3 * rules::SCAN_RECHARGE_TICKS);
    assert_eq!(
        world.scan_ledger(0).charges,
        rules::SCAN_MAX_CHARGES,
        "never more"
    );
}

#[test]
fn using_a_charge_starts_a_wait_only_when_none_runs_and_a_running_wait_goes_on() {
    let mut world = duel();
    let second = first_charge(&world) + rules::SCAN_RECHARGE_TICKS;
    run_to(&mut world, second);
    let there = Vec2::from_ints(8000, 8000);
    let started = world.clock();
    give(&mut world, RADIANT, scan(there));
    assert_eq!(
        world.scan_ledger(0),
        ScanLedger {
            charges: 1,
            next_charge: Some(started + rules::SCAN_RECHARGE_TICKS),
        },
        "a full side starts its wait at use"
    );
    run(&mut world, 100);
    give(&mut world, RADIANT, scan(there));
    assert_eq!(
        world.scan_ledger(0),
        ScanLedger {
            charges: 0,
            next_charge: Some(started + rules::SCAN_RECHARGE_TICKS),
        },
        "a wait that runs is not started again"
    );
    assert_eq!(
        world.validate_order(RADIANT, None, &scan(there)),
        Err(RejectReason::NoScanCharge)
    );
    run_to(&mut world, started + rules::SCAN_RECHARGE_TICKS);
    assert_eq!(world.scan_ledger(0).charges, 1);
    assert_eq!(world.validate_order(RADIANT, None, &scan(there)), Ok(()));
}

#[test]
fn the_charges_are_the_sides_whichever_seat_spends_them_and_the_other_side_keeps_its_own() {
    let mut cfg = mid_config();
    cfg.cheats = true;
    let mut world = World::for_match(&cfg, cfg.rng());
    world.settle();
    let second = first_charge(&world) + rules::SCAN_RECHARGE_TICKS;
    run_to(&mut world, second);
    let there = Vec2::from_ints(8000, 8000);
    give(&mut world, RADIANT, scan(there));
    give(&mut world, RADIANT, scan(there));
    assert_eq!(world.scan_ledger(0).charges, 0);
    assert_eq!(world.scan_ledger(1).charges, rules::SCAN_MAX_CHARGES);
    assert_eq!(
        world.validate_order(RADIANT, None, &scan(there)),
        Err(RejectReason::NoScanCharge)
    );
    assert_eq!(world.validate_order(DIRE, None, &scan(there)), Ok(()));
}

#[test]
fn an_unseated_slot_may_not_scan_for_either_side() {
    let mut world = duel();
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    assert_eq!(
        world.validate_order(SlotId(9), None, &scan(Vec2::from_ints(1, 1))),
        Err(RejectReason::HeroDead)
    );
    assert_eq!(world.scan_ledger(0).charges, 1, "nothing was spent");
}

#[test]
fn a_seat_whose_hero_lies_dead_may_scan() {
    let mut world = duel();
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    let hero = hero_of(&world, RADIANT);
    world.bury(vec![(hero, None)], &mut Vec::new());
    assert!(!world.alive(hero));
    give(&mut world, RADIANT, scan(Vec2::from_ints(8000, 8000)));
    assert_eq!(answers(&world).len(), 1, "the scan stands");
}

#[test]
fn a_spam_of_scans_in_one_tick_spends_the_charges_and_refuses_the_rest() {
    let mut world = duel();
    let second = first_charge(&world) + rules::SCAN_RECHARGE_TICKS;
    run_to(&mut world, second);
    let orders: Vec<Command> = (0..5)
        .map(|n| Command {
            seq: n,
            ..command(RADIANT, scan(Vec2::from_ints(8000 + 10 * n as i32, 8000)))
        })
        .collect();
    world.advance_checked(&orders);
    assert_eq!(answers(&world).len(), 2, "two charges, two scans");
    let refused: Vec<(u32, RejectReason)> = world
        .refused
        .iter()
        .map(|refused| (refused.seq, refused.reason))
        .collect();
    assert_eq!(
        refused,
        [
            (2, RejectReason::NoScanCharge),
            (3, RejectReason::NoScanCharge),
            (4, RejectReason::NoScanCharge)
        ]
    );
}

#[test]
fn a_scan_aimed_off_the_map_is_kept_on_it() {
    let order = bota_core::order_on_map(scan(Vec2::from_ints(-20_000, 30_000)));
    let Order::Scan { at } = order else {
        panic!("still a scan");
    };
    let size = Fixed::from_int(rules::MAP_SIZE);
    assert_eq!((at.x, at.y), (Fixed::ZERO, size));
}

#[test]
fn a_scan_finds_a_visible_hero() {
    let mut world = duel();
    let enemy_at = teleport(&mut world, DIRE, DIRE_BASE);
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    teleport(&mut world, RADIANT, enemy_at + Vec2::from_ints(-300, 0));
    assert!(
        world
            .visibility
            .get(hero_of(&world, DIRE))
            .is_some_and(|seen| seen.by(Team::Radiant))
    );
    give(&mut world, RADIANT, scan(enemy_at));
    assert!(answer(&world));
}

#[test]
fn a_scan_finds_a_hero_in_the_fog() {
    let (world, _) = scanned_enemy(Vec2::from_ints(100, 0));
    assert!(
        !world
            .visibility
            .get(hero_of(&world, DIRE))
            .is_some_and(|seen| seen.by(Team::Radiant)),
        "the enemy is in the fog"
    );
    assert!(answer(&world));
}

#[test]
fn a_scan_finds_an_invisible_hero() {
    let mut world = duel();
    let enemy_at = teleport(&mut world, DIRE, DIRE_BASE);
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    teleport(&mut world, RADIANT, enemy_at + Vec2::from_ints(-300, 0));
    let enemy = hero_of(&world, DIRE);
    put_on(
        &mut world,
        enemy,
        ModifierKind::RuneInvisibility,
        60 * rules::TICKS_PER_SECOND,
    );
    run(&mut world, 2 * bota_core::INVISIBILITY_FADE_TICKS);
    assert!(
        !world
            .visibility
            .get(enemy)
            .is_some_and(|seen| seen.by(Team::Radiant)),
        "the invisible hero is not seen even by a hero beside it"
    );
    give(&mut world, RADIANT, scan(enemy_at));
    assert!(answer(&world));
}

#[test]
fn a_scan_finds_an_illusion() {
    let mut world = duel();
    let enemy_at = teleport(&mut world, DIRE, DIRE_BASE);
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    let enemy = hero_of(&world, DIRE);
    world.make_rune_illusions(enemy);
    // The real hero walks away: only its illusions stand in the area.
    let away = teleport(&mut world, DIRE, RADIANT_BASE);
    assert!(!away.within(enemy_at, rules::units(rules::SCAN_RADIUS)));
    assert!(
        world.entities.iter().any(|entity| world
            .illusion
            .get(entity)
            .is_some_and(|made| made.of == enemy)),
        "illusions of the enemy hero stand"
    );
    give(&mut world, RADIANT, scan(enemy_at));
    assert!(answer(&world));
}

#[test]
fn a_scan_does_not_find_a_dead_hero() {
    let (mut world, enemy_at) = scanned_enemy(Vec2::from_ints(0, 0));
    assert!(answer(&world));
    let enemy = hero_of(&world, DIRE);
    world.bury(vec![(enemy, None)], &mut Vec::new());
    run(&mut world, rules::SCAN_LOOK_TICKS);
    assert!(!answer(&world), "the next look misses it: {enemy_at:?}");
}

#[test]
fn a_scan_does_not_find_a_hero_waiting_to_respawn() {
    let mut world = duel();
    let enemy_at = teleport(&mut world, DIRE, DIRE_BASE);
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    let enemy = hero_of(&world, DIRE);
    world.bury(vec![(enemy, None)], &mut Vec::new());
    assert!(!world.alive(enemy));
    assert!(world.seat(DIRE).is_some_and(|seat| seat.respawn_left > 0));
    give(&mut world, RADIANT, scan(enemy_at));
    assert!(!answer(&world));
}

#[test]
fn a_scan_does_not_find_an_invulnerable_hero() {
    let mut world = duel();
    let enemy_at = teleport(&mut world, DIRE, DIRE_BASE);
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    let enemy = hero_of(&world, DIRE);
    put_on(
        &mut world,
        enemy,
        ModifierKind::FountainInvulnerable,
        60 * rules::TICKS_PER_SECOND,
    );
    world.step();
    assert!(
        world
            .stats
            .get(enemy)
            .is_some_and(|stats| stats.invulnerable)
    );
    give(&mut world, RADIANT, scan(enemy_at));
    assert!(!answer(&world));
}

#[test]
fn a_scan_does_not_find_the_scanning_sides_own_heroes() {
    let mut world = duel();
    let own_at = teleport(&mut world, RADIANT, Vec2::from_ints(5000, 9000));
    teleport(&mut world, DIRE, Vec2::from_ints(9000, 3000));
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    give(&mut world, RADIANT, scan(own_at));
    assert!(!answer(&world));
}

#[test]
fn a_scan_does_not_find_couriers_creeps_wards_or_buildings() {
    let mut world = duel();
    // The Dire hero leaves its base, which keeps its fountain, its Ancient, its towers and
    // barracks, its courier and whatever creeps and neutrals stand there.
    teleport(&mut world, DIRE, Vec2::from_ints(3000, 3000));
    let base = map_of(MAP2_ID).fountains[1];
    let ready = first_charge(&world);
    run_to(&mut world, ready);
    let standing = world
        .entities
        .iter()
        .filter(|&entity| world.team.get(entity) == Some(&Team::Dire))
        .filter(|&entity| {
            world
                .pos(entity)
                .is_some_and(|pos| pos.within(base, rules::units(rules::SCAN_RADIUS)))
        })
        .count();
    assert!(standing >= 2, "Dire things stand in the area: {standing}");
    give(&mut world, RADIANT, scan(base));
    assert!(!answer(&world));
}

#[test]
fn a_scan_counts_a_hero_by_the_distance_of_its_origin_less_the_bound_of_its_hull() {
    // Shadow Fiend's hull is 24 units: it is caught within 900 + 24, the edge included.
    assert_eq!(rules::HERO_BOUND, 24);
    for (apart, found) in [(924, true), (925, false)] {
        let (world, _) = scanned_enemy(Vec2::from_ints(apart, 0));
        assert_eq!(answer(&world), found, "{apart} units from the point");
    }
}

#[test]
fn a_scan_has_no_special_area_for_fountains_or_the_roshan_pit() {
    // Roshan's pit on a map that has one: the map of a full game, not Solo Mid's.
    let pit = map_of(bota_proto::MapId(0))
        .boss_zones
        .first()
        .expect("a pit");
    let count = pit.corners.len() as i64;
    let mean = |raw: fn(&Vec2) -> i32| {
        (pit.corners
            .iter()
            .map(|corner| i64::from(raw(corner)))
            .sum::<i64>()
            / count) as i32
    };
    let middle = Vec2 {
        x: Fixed {
            raw: mean(|corner| corner.x.raw),
        },
        y: Fixed {
            raw: mean(|corner| corner.y.raw),
        },
    };
    for place in [DIRE_BASE, middle] {
        let mut world = duel();
        let at = teleport(&mut world, DIRE, place);
        let ready = first_charge(&world);
        run_to(&mut world, ready);
        // A hero risen in its fountain is invulnerable and so not counted; this one never fell.
        give(&mut world, RADIANT, scan(at));
        assert!(answer(&world), "a hero at {at:?}");
    }
}

#[test]
fn the_answer_is_looked_for_again_every_second_for_nine_looks_from_the_first_to_the_eighth_second()
{
    let (mut world, enemy_at) = scanned_enemy(Vec2::from_ints(0, 0));
    // The order's own tick has run since its look.
    let placed = world.tick - 1;
    let mut told = vec![answers(&world)[0]];
    for second in 1..=8 {
        // The hero steps out of the area in the third second and back in the sixth.
        if second == 3 {
            let away = teleport(&mut world, DIRE, enemy_at - Vec2::from_ints(2500, 0));
            assert!(!away.within(enemy_at, rules::units(rules::SCAN_RADIUS)));
        }
        if second == 6 {
            let back = teleport(&mut world, DIRE, enemy_at);
            assert!(back.within(enemy_at, rules::units(rules::SCAN_RADIUS)));
        }
        run_to(&mut world, placed + second * rules::SCAN_LOOK_TICKS);
        told.extend(answers(&world).first().copied());
    }
    // Looks are made at 0, 1, ..., 8 seconds, the answer of each the one told until the next:
    // the thinker's think comes before its expiry at 8, so the ninth look is made and told.
    let left = |second: u32| rules::SCAN_DURATION_TICKS - second * rules::SCAN_LOOK_TICKS;
    assert_eq!(
        told,
        [
            (Team::Radiant, true, left(0) - 1),
            (Team::Radiant, true, left(1)),
            (Team::Radiant, true, left(2)),
            (Team::Radiant, false, left(3)),
            (Team::Radiant, false, left(4)),
            (Team::Radiant, false, left(5)),
            (Team::Radiant, true, left(6)),
            (Team::Radiant, true, left(7)),
            (Team::Radiant, true, left(8)),
        ]
    );
    world.step();
    assert!(answers(&world).is_empty(), "no scan after the ninth look");
}

#[test]
fn the_scanning_side_is_told_its_charges_and_scans_and_the_other_side_nothing() {
    let (mut world, enemy_at) = scanned_enemy(Vec2::from_ints(0, 0));
    let events = world.step();
    let own = scan_events(&events, Some(Team::Radiant));
    assert_eq!(own.len(), 1);
    let EventKind::Scan {
        team,
        charges,
        radars,
        ..
    } = &own[0]
    else {
        panic!("a scan fact")
    };
    assert_eq!((*team, *charges), (Team::Radiant, 0));
    assert_eq!(radars.len(), 1);
    assert_eq!(radars[0].at, enemy_at);
    assert!(radars[0].found);
    // One tick ran with the order, one more since.
    assert_eq!(radars[0].ticks_left, rules::SCAN_DURATION_TICKS - 2);
    assert!(
        scan_events(&events, Some(Team::Dire)).is_empty(),
        "nothing of it to the other side"
    );
    assert_eq!(
        scan_events(&events, None).len(),
        1,
        "a spectator is told of the side that scanned"
    );
}

#[test]
fn what_the_other_side_is_told_never_depends_on_what_a_scan_found() {
    let mut told_to_dire = Vec::new();
    for offset in [Vec2::from_ints(0, 0), Vec2::from_ints(5000, 0)] {
        let (mut world, _) = scanned_enemy(offset);
        let mut stream = Vec::new();
        for _ in 0..(rules::SCAN_DURATION_TICKS + 30) {
            let events = world.step();
            stream.push(visible_events(&events, Some(Team::Dire)));
        }
        told_to_dire.push((stream, world.hash()));
    }
    assert_eq!(
        told_to_dire[0].0, told_to_dire[1].0,
        "the events of the scanned side are the same whether the scan found its hero or not"
    );
    assert!(told_to_dire[0].0.iter().all(|events| {
        events
            .iter()
            .all(|kind| !matches!(kind, EventKind::Scan { .. }))
    }));
}

#[test]
fn a_scan_is_a_function_of_the_seed_and_the_orders() {
    let hash = |scans: bool| {
        let (mut world, _) = if scans {
            scanned_enemy(Vec2::from_ints(0, 0))
        } else {
            let mut world = duel();
            teleport(&mut world, DIRE, DIRE_BASE);
            let ready = first_charge(&world);
            run_to(&mut world, ready);
            world.step();
            (world, DIRE_BASE)
        };
        run(&mut world, 2 * rules::SCAN_DURATION_TICKS);
        world.hash()
    };
    assert_eq!(hash(true), hash(true));
    assert_ne!(
        hash(true),
        hash(false),
        "the fingerprint holds what a side has spent"
    );
}

#[test]
fn a_refresh_cheat_gives_each_side_one_charge_on_the_next_tick_and_restarts_the_wait() {
    let mut world = duel();
    assert_eq!(world.scan_ledger(0).charges, 0);
    let at = world.clock();
    give(
        &mut world,
        RADIANT,
        Order::Cheat {
            cheat: Cheat::Refresh,
        },
    );
    for side in [0, 1] {
        assert_eq!(
            world.scan_ledger(side),
            ScanLedger {
                charges: 1,
                next_charge: Some(at + rules::SCAN_RECHARGE_TICKS),
            },
            "side {side}: one charge, and a fresh wait from it"
        );
    }
    let at = world.clock();
    give(
        &mut world,
        RADIANT,
        Order::Cheat {
            cheat: Cheat::Refresh,
        },
    );
    give(
        &mut world,
        RADIANT,
        Order::Cheat {
            cheat: Cheat::Refresh,
        },
    );
    assert_eq!(
        world.scan_ledger(0),
        ScanLedger {
            charges: rules::SCAN_MAX_CHARGES,
            next_charge: None,
        },
        "the most, and no wait; from {at}"
    );
    give(
        &mut world,
        RADIANT,
        Order::Cheat {
            cheat: Cheat::Refresh,
        },
    );
    assert_eq!(
        world.scan_ledger(0).charges,
        rules::SCAN_MAX_CHARGES,
        "never past it"
    );
}

/// What a seat is told in one step of an in-process match: its events, and the reasons its
/// orders were turned down.
fn told(stream: &[bota_proto::ServerMsg]) -> (Vec<EventKind>, Vec<RejectReason>) {
    let mut events = Vec::new();
    let mut refused = Vec::new();
    for msg in stream {
        match msg {
            bota_proto::ServerMsg::Events { events: told, .. } => events.extend(told.clone()),
            bota_proto::ServerMsg::OrderRejected { reason, .. } => refused.push(*reason),
            _ => {}
        }
    }
    (events, refused)
}

/// The Scan an event stream tells of, if it tells of one: its side, charges and scans.
fn scan_of(events: &[EventKind]) -> Option<(Team, u8, usize, bool)> {
    events.iter().find_map(|kind| match kind {
        EventKind::Scan {
            team,
            charges,
            radars,
            ..
        } => Some((
            *team,
            *charges,
            radars.len(),
            radars.iter().any(|radar| radar.found),
        )),
        _ => None,
    })
}

#[test]
fn a_scan_reaches_its_side_and_a_replay_and_never_the_other_side_of_a_real_match() {
    use crate::game::OrderDelay;
    use crate::local::LocalMatch;
    use bota_proto::{OutgoingOrder, ReplayRecord, ServerMsg};

    let mut cfg = mid_config();
    cfg.cheats = true;
    let ready = map_of(cfg.map).schedule().scan_first_charge();
    let (mut local, _) = LocalMatch::new_recorded(&cfg, &[OrderDelay::NONE]).expect("a match");
    let send = |seq: u32, tick: u32, order: Order| OutgoingOrder {
        seq,
        unit: None,
        order,
        seen: tick,
        queue: false,
    };
    // Dire teleports to the far corner of its base; nobody scans before the first charge.
    let teleport = Order::Cheat {
        cheat: Cheat::Teleport { at: DIRE_BASE },
    };
    local.step(&[&[], &[send(1, 1, teleport)]]).expect("a step");
    let mut early = Vec::new();
    while local.tick() < ready - 1 {
        let streams = local.step(&[&[], &[]]).expect("a step");
        early.extend(streams.iter().flat_map(|stream| told(stream).0));
    }
    assert_eq!(
        scan_of(&early),
        None,
        "a match nobody scans in tells nothing of Scan"
    );
    let enemy_at = local
        .world()
        .pos(local.hero_of(1).expect("standing"))
        .expect("standing");
    // Scan too early: refused, nothing spent.
    let too_early = local
        .step(&[&[send(2, local.tick(), scan(enemy_at))], &[]])
        .expect("a step");
    assert_eq!(told(&too_early[0]).1, [RejectReason::NoScanCharge]);
    assert_eq!(scan_of(&told(&too_early[0]).0), None);
    // At the first charge the order stands, and the answer is told to Radiant alone.
    let streams = local
        .step(&[&[send(3, local.tick(), scan(enemy_at))], &[]])
        .expect("a step");
    let own = told(&streams[0]);
    assert_eq!(own.1, []);
    assert_eq!(scan_of(&own.0), Some((Team::Radiant, 0, 1, true)));
    assert_eq!(
        scan_of(&told(&streams[1]).0),
        None,
        "the scanned side is told nothing"
    );
    let records = local.take_records();
    let spectated: Vec<EventKind> = records
        .iter()
        .filter_map(|record| match record {
            ReplayRecord::Msg(ServerMsg::Events { events, .. }) => Some(events.clone()),
            _ => None,
        })
        .flatten()
        .filter(|kind| {
            matches!(
                kind,
                EventKind::Scan {
                    team: Team::Radiant,
                    ..
                }
            )
        })
        .collect();
    assert!(
        !spectated.is_empty(),
        "the replay holds what the scanning side was told"
    );
    assert!(
        records.iter().any(|record| matches!(
            record,
            ReplayRecord::Orders { orders, .. } if orders.iter().any(|given| matches!(given.order, Order::Scan { .. }))
        )),
        "and the accepted order"
    );
}

/// What a seat's client holds of its side's Scan after a step: the restated fact when the stream
/// has told one, else what the clock alone says.
fn side_known(world: &World, events: &[Event]) -> bota_client_core::ScanSide {
    let told = scan_events(events, Some(Team::Radiant))
        .into_iter()
        .find_map(|kind| match kind {
            EventKind::Scan {
                team: Team::Radiant,
                charges,
                next_charge_in,
                radars,
            } => Some(bota_client_core::ScanSide {
                ledger: ScanLedger {
                    charges,
                    next_charge: next_charge_in.map(|left| world.clock() + left),
                },
                radars,
            }),
            _ => None,
        });
    told.unwrap_or_else(|| bota_client_core::ScanSide {
        ledger: ScanLedger::untouched(world.schedule(), world.clock()),
        radars: Vec::new(),
    })
}

#[test]
fn what_the_scan_facts_prove_of_admission_is_what_the_server_admits() {
    use bota_client_core::order_legality::{UnknownReason, Verdict, scan_legality};
    let mut world = duel();
    let ready = first_charge(&world);
    let there = Vec2::from_ints(8000, 8000);
    // One tick on, what the client holds and the server's own answer are compared.
    let observe = |world: &mut World| {
        let events = world.step();
        let proven = scan_legality(Some(&side_known(world, &events)));
        let server = world.validate_order(RADIANT, None, &scan(there));
        let agree = match (proven, server) {
            (Verdict::Legal, Ok(())) => true,
            (Verdict::Illegal(why), Err(refused)) => why == refused,
            _ => false,
        };
        assert!(
            agree,
            "tick {}: the facts prove {proven:?}, the server says {server:?}",
            world.tick
        );
        server.is_ok()
    };
    run_to(&mut world, ready - 2);
    assert!(!observe(&mut world), "no charge a tick before the first");
    assert!(observe(&mut world), "the first charge is there");
    give(&mut world, RADIANT, scan(there));
    assert!(!observe(&mut world), "spent, and the wait runs");
    run_to(&mut world, ready + rules::SCAN_RECHARGE_TICKS - 1);
    assert!(observe(&mut world), "the next charge, a wait later");
    give(&mut world, RADIANT, scan(there));
    assert!(!observe(&mut world), "spent again");
    assert_eq!(
        scan_legality(None),
        Verdict::Unknown(UnknownReason::HiddenState),
        "before the first batch nothing is proven"
    );
}
