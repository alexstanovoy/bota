//! The client library's session folds what a server sends and nothing else.

use bota_client_core::{
    AckPolicy, ClientSession, Ingested, IssuedOrder, MatchStatic, SessionOptions,
};
use bota_core::{map_of, rules};
use bota_proto::{
    AbilitySlot, EntityId, EventKind, Fixed, HeroId, ItemId, ItemSlot, MatchInfo, Order,
    OutgoingOrder, ServerMsg, SkippedTick, SlotId, Target, Team, UnitKind, Vec2,
};

use super::local::mid_config;
use crate::game::{OrderDelay, UnitOrder};
use crate::local::LocalMatch;

const TICKS: u32 = rules::FIRST_WAVE_TICK + 4000;

/// A two-seat map-2 match with both heroes attack-moving at the other's
/// fountain: every message each seat was sent, in order.
fn play() -> Vec<Vec<ServerMsg>> {
    let cfg = mid_config();
    let map = map_of(cfg.map);
    let (mut local, mut seats) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    for _ in 0..TICKS {
        let tick = local.tick();
        let order = |slot: usize| OutgoingOrder {
            seq: tick,
            unit: None,
            order: Order::Attack {
                target: Target::Pos(map.fountains[1 - slot]),
            },
            seen: tick,
        };
        let step = local
            .step(&[Some(order(0)), Some(order(1))])
            .expect("a step");
        let over = step[0]
            .iter()
            .any(|message| matches!(message, ServerMsg::MatchOver { .. }));
        for (stream, messages) in seats.iter_mut().zip(step) {
            stream.extend(messages);
        }
        if over {
            break;
        }
    }
    seats
}

fn session_for(info: &MatchInfo, slot: u8) -> ClientSession {
    let options = SessionOptions {
        ack: AckPolicy::Never,
        ..SessionOptions::default()
    };
    let statics = MatchStatic::new(info).expect("valid terms");
    ClientSession::seat(statics, SlotId(slot), Some(HeroId::SHADOW_FIEND), options).expect("a seat")
}

fn info_of(stream: &[ServerMsg]) -> &MatchInfo {
    match &stream[0] {
        ServerMsg::MatchStart { info } => info,
        other => panic!("a stream starts with MatchStart, not {other:?}"),
    }
}

#[test]
fn two_sessions_fed_one_stream_hold_equal_state() {
    let seats = play();
    let info = info_of(&seats[0]);
    let (mut first, mut second) = (session_for(info, 0), session_for(info, 0));
    for message in &seats[0][1..] {
        let a = first.ingest(message.clone(), &mut ()).expect("folds");
        let b = second.ingest(message.clone(), &mut ()).expect("folds");
        assert_eq!(a, b);
    }
    assert_eq!(first, second);
    let tracked = first.entities().len();
    assert!(tracked > 10, "a match is seen: {tracked} entities");
}

#[test]
fn a_stream_with_skipped_ticks_folds_every_death_and_damage() {
    const EVERY: u32 = 3;
    let seats = play();
    let info = info_of(&seats[0]);
    let mut every = session_for(info, 0);
    let mut slow = session_for(info, 0);
    let mut skipped: Vec<SkippedTick> = Vec::new();
    let mut told_by_skips = 0;
    let mut snapshot = None;
    for message in &seats[0][1..] {
        every.ingest(message.clone(), &mut ()).expect("folds");
        match message {
            ServerMsg::Snapshot { view } => snapshot = Some(view.clone()),
            ServerMsg::Events { tick, events } if tick % EVERY == 0 => {
                let mut view = snapshot.take().expect("events follow their snapshot");
                told_by_skips += skipped
                    .iter()
                    .flat_map(|tick| &tick.events)
                    .filter(|e| matches!(e, EventKind::Died { .. } | EventKind::Damaged { .. }))
                    .count();
                view.skipped_ticks = std::mem::take(&mut skipped);
                slow.ingest(ServerMsg::Snapshot { view }, &mut ())
                    .expect("folds");
                slow.ingest(message.clone(), &mut ()).expect("folds");
                assert_eq!(slow.recent_events(), every.recent_events(), "tick {tick}");
            }
            ServerMsg::Events { tick, events } => skipped.push(SkippedTick {
                tick: *tick,
                events: events.clone(),
            }),
            _ => {}
        }
    }
    assert!(told_by_skips > 0, "the skipped ticks carried a fight");
}

#[test]
fn a_refused_order_is_rolled_back_in_the_ledger() {
    let cfg = mid_config();
    let (mut local, mut seats) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let mut session = session_for(info_of(&seats[0]), 0);
    for message in seats[0].drain(1..) {
        session.ingest(message, &mut ()).expect("folds");
    }
    let nowhere = EntityId {
        idx: 9_000,
        generation: 1,
    };
    let issued = IssuedOrder {
        unit: None,
        order: Order::Attack {
            target: Target::Unit(nowhere),
        },
    };
    let sent = session.issue(issued).expect("a snapshot is held");
    assert_eq!((sent.seq, sent.seen), (1, local.tick()));
    assert!(session.ledger().repeats(issued));
    let step = local.step(&[Some(sent), None]).expect("a step");
    let mut refused = false;
    for message in step[0].clone() {
        refused |= matches!(
            session.ingest(message, &mut ()).expect("folds"),
            Ingested::Rejected { seq: 1, .. }
        );
    }
    assert!(refused, "the server refused an order at nobody");
    assert!(!session.ledger().repeats(issued), "nothing stands any more");
}

/// Gives seat 0 one order through its session and plays `ticks` ticks on,
/// the session folding every message.
fn give(local: &mut LocalMatch, session: &mut ClientSession, order: Order, ticks: u32) {
    let sent = session
        .issue(IssuedOrder::hero(order))
        .expect("a snapshot is held");
    let mut first = Some(sent);
    for _ in 0..ticks {
        let step = local.step(&[first.take(), None]).expect("a step");
        for message in step.into_iter().next().expect("seat 0") {
            let folded = session.ingest(message, &mut ()).expect("folds");
            assert!(
                !matches!(folded, Ingested::Rejected { .. }),
                "{order:?} refused"
            );
        }
    }
}

#[test]
fn the_ledger_keeps_a_standing_walk_exactly_while_the_server_does() {
    let cfg = mid_config();
    let (mut local, mut seats) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    let mut session = session_for(info_of(&seats[0]), 0);
    for message in seats[0].drain(1..) {
        session.ingest(message, &mut ()).expect("folds");
    }
    let hero = local.world().seats[0].unit.expect("a hero");
    let idle = Order::Move {
        target: Target::None,
    };
    give(
        &mut local,
        &mut session,
        idle,
        rules::SCROLL_FIRST_WAIT_TICKS,
    );
    super::support::hand_item(local.world_mut(), hero, ItemId::HEALING_SALVE, 1);
    local.world_mut().health.get_mut(hero).expect("health").hp = Fixed::from_int(300);
    let home = local.world().transform.get(hero).expect("standing").pos;
    let goal = home + Vec2::from_ints(1_500, 1_500);
    let walk = Order::Move {
        target: Target::Pos(goal),
    };
    let walks_on = |local: &LocalMatch, session: &ClientSession| {
        let server = local.world().orders.get(hero).expect("orders").current;
        let ledger = session
            .ledger()
            .standing(None)
            .map(|(_, issued)| issued.order);
        (
            server == UnitOrder::Move { pos: goal },
            ledger == Some(walk),
        )
    };
    give(&mut local, &mut session, walk, 5);
    assert_eq!(walks_on(&local, &session), (true, true));
    let drink = Order::Use {
        slot: ItemSlot(0),
        target: Target::Unit(crate::game::wire_id(hero)),
    };
    give(&mut local, &mut session, drink, 5);
    assert!(local.world().inventory.get(hero).expect("a bag").slots[0].is_none());
    assert_eq!(walks_on(&local, &session), (true, true), "a salve");
    let near = AbilitySlot(0);
    give(&mut local, &mut session, Order::Learn { slot: near }, 1);
    let raze = Order::Cast {
        slot: near,
        target: Target::None,
    };
    give(&mut local, &mut session, raze, 5);
    assert_eq!(walks_on(&local, &session), (true, true), "a raze");
    let world = local.world();
    let tower = world
        .entities
        .iter()
        .find(|&unit| {
            world.kind.get(unit) == Some(&UnitKind::Tower)
                && world.team.get(unit) == Some(&Team::Radiant)
        })
        .expect("a tower of its own");
    let read = Order::Use {
        slot: ItemSlot::TOWN_PORTAL,
        target: Target::Unit(crate::game::wire_id(tower)),
    };
    give(
        &mut local,
        &mut session,
        read,
        rules::SCROLL_CHANNEL_TICKS + 2,
    );
    let server = local.world().orders.get(hero).expect("orders").current;
    assert_eq!(server, UnitOrder::Idle, "a scroll's landing ends the walk");
    assert_eq!(
        session.ledger().standing(None),
        None,
        "and the ledger agrees"
    );
}
