//! The client library's session folds what a server sends and nothing else.

use bota_client_core::{
    AckPolicy, ClientSession, Ingested, IssuedOrder, MatchStatic, Perspective, SessionOptions,
};
use bota_core::{map_of, rules};
use bota_proto::{
    EntityId, EventKind, HeroId, MatchInfo, Order, ServerMsg, SkippedTick, SlotId, Target,
};

use super::local::mid_config;
use crate::game::OrderDelay;
use crate::local::{LocalMatch, LocalOrder};

const TICKS: u32 = rules::FIRST_WAVE_TICK + 4000;

/// A two-seat map-2 match with both heroes attack-moving at the other's
/// fountain: every message each seat was sent, in order.
fn play() -> Vec<Vec<ServerMsg>> {
    let cfg = mid_config();
    let map = map_of(cfg.map);
    let (mut local, mut seats) = LocalMatch::new(&cfg, &[OrderDelay::NONE]).expect("a match");
    for _ in 0..TICKS {
        let tick = local.tick();
        let order = |slot: usize| LocalOrder {
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
        perspective: Perspective::Seat(SlotId(slot)),
        hero: Some(HeroId::SHADOW_FIEND),
        ack: AckPolicy::Never,
        forget_after_ticks: 480,
    };
    ClientSession::new(MatchStatic::new(info).expect("valid terms"), options).expect("a seat")
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
    let step = local
        .step(&[
            Some(LocalOrder {
                seq: sent.seq,
                unit: sent.unit,
                order: sent.order,
                seen: sent.seen,
            }),
            None,
        ])
        .expect("a step");
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
