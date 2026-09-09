use std::io::Cursor;

use bota_proto::{
    EventKind, MatchStats, Order, ReplayRecord, ServerMsg, SlotId, SlotOrder, Target, Team,
};

use super::fixtures::{counted, match_start, message, snapshot, wire};
use crate::replay_play::{MAX_RECORDS_PER_POLL, ReplayPlayer};
use crate::state::{App, Source};

fn started_player(rate: u16, ticks: u32) -> ReplayPlayer {
    let mut records = vec![match_start(rate)];
    records.extend((0..=ticks).map(snapshot));
    let mut player = ReplayPlayer::from_reader(Cursor::new(wire(&records)));
    assert_eq!(player.poll(60.0).len(), 2);
    assert!(
        player.poll(60.0).is_empty(),
        "first render time is not replay time"
    );
    player
}

#[test]
fn thirty_hz_clock_releases_exactly_one_snapshot_per_tick() {
    let mut player = started_player(30, 30);
    for tick in 1..=30 {
        assert_eq!(player.poll(1.0 / 30.0), vec![message(snapshot(tick))]);
    }
    assert!(player.finished());
}

#[test]
fn clock_uses_the_recorded_rate_and_zero_rate_falls_back_to_one() {
    for rate in [0, 1, 10, 60, 120] {
        let mut player = started_player(rate, 10);
        assert_eq!(
            player.poll(1.0 / f32::from(rate.max(1))),
            vec![message(snapshot(1))]
        );
    }
}

#[test]
fn fractional_frame_times_accumulate_without_releasing_future_snapshots() {
    let mut player = started_player(30, 4);
    for tick in 1..=4 {
        assert!(player.poll(1.0 / 60.0).is_empty());
        assert_eq!(player.poll(1.0 / 60.0), vec![message(snapshot(tick))]);
    }
}

#[test]
fn pause_freezes_elapsed_time_and_step_queues_one_tick_without_reading() {
    let (reader, bytes, calls) = counted(wire(&[snapshot(0), snapshot(1), snapshot(2)]), 1);
    let mut player = ReplayPlayer::from_reader(reader);
    player.poll(0.0);
    player.poll(0.0);
    player.paused = true;
    assert_eq!(player.status(), "paused");
    assert!(player.poll(60.0).is_empty());
    let before = (bytes.get(), calls.get());

    player.advance_ticks(1.0);

    assert_eq!((bytes.get(), calls.get()), before);
    assert_eq!(player.poll(60.0), vec![message(snapshot(1))]);
    assert!(player.poll(60.0).is_empty());
    player.paused = false;
    assert_eq!(player.poll(1.0 / 30.0), vec![message(snapshot(2))]);
}

#[test]
fn speed_changes_scale_elapsed_time_without_discarding_ticks() {
    let mut player = started_player(30, 20);
    player.speed = 0.25;
    for _ in 0..3 {
        assert!(player.poll(1.0 / 30.0).is_empty());
    }
    assert_eq!(player.poll(1.0 / 30.0), vec![message(snapshot(1))]);
    player.speed = 2.0;
    assert_eq!(
        player.poll(1.0 / 30.0),
        vec![message(snapshot(2)), message(snapshot(3))]
    );
    player.speed = 16.0;
    assert_eq!(
        player.poll(1.0 / 30.0),
        (4..=19)
            .map(|tick| message(snapshot(tick)))
            .collect::<Vec<_>>()
    );
}

#[test]
fn first_snapshot_anchors_a_nonzero_recording_start() {
    let mut player = ReplayPlayer::from_reader(Cursor::new(wire(&[snapshot(900), snapshot(901)])));
    assert_eq!(player.poll(60.0), vec![message(snapshot(900))]);
    assert!(player.poll(60.0).is_empty());
    assert_eq!(player.poll(1.0 / 30.0), vec![message(snapshot(901))]);
}

fn tick_records(tick: u32) -> Vec<ReplayRecord> {
    vec![
        snapshot(tick),
        ReplayRecord::Msg(ServerMsg::Events {
            tick,
            events: vec![EventKind::StructureDestroyed {
                unit: bota_proto::EntityId {
                    idx: tick,
                    generation: 0,
                },
                team: Team::Dire,
            }],
        }),
        ReplayRecord::Orders {
            tick,
            orders: vec![SlotOrder {
                slot: SlotId(0),
                unit: None,
                order: Order::Move {
                    target: Target::None,
                },
            }],
        },
    ]
}

#[test]
fn snapshots_events_orders_and_match_end_remain_in_file_order_across_batches() {
    let mut records = vec![match_start(30)];
    for tick in 0..100 {
        records.extend(tick_records(tick));
    }
    records.push(ReplayRecord::Msg(ServerMsg::MatchOver {
        winner: Team::Radiant,
        stats: MatchStats {
            duration: 99,
            slots: Vec::new(),
        },
    }));
    let mut player = ReplayPlayer::from_reader(Cursor::new(wire(&records)));
    let mut received = player.poll(0.0);
    player.paused = true;
    player.advance_ticks(100.0);
    for _ in 0..records.len() {
        let batch = player.poll(0.0);
        assert!(batch.len() <= MAX_RECORDS_PER_POLL);
        received.extend(batch);
        if player.finished() {
            break;
        }
    }
    assert!(player.finished());
    assert!(player.error().is_none());
    assert_eq!(
        received,
        records.into_iter().map(message).collect::<Vec<_>>()
    );
}

#[test]
fn future_orders_block_following_events_until_their_tick() {
    let mut records = vec![snapshot(0)];
    let future = tick_records(2);
    records.extend_from_slice(&future[2..]);
    records.extend_from_slice(&future[1..2]);
    let mut player = ReplayPlayer::from_reader(Cursor::new(wire(&records)));
    player.poll(0.0);
    assert!(player.poll(60.0).is_empty());
    player.advance_ticks(1.0);
    assert!(player.poll(0.0).is_empty());
    player.advance_ticks(1.0);
    assert_eq!(
        player.poll(0.0),
        vec![message(future[2].clone()), message(future[1].clone())]
    );
}

#[test]
fn stepping_delivers_orders_to_the_app_in_the_same_batch_as_their_events() {
    let mut records = vec![match_start(30)];
    records.extend(tick_records(0));
    records.extend(tick_records(1));
    let player = ReplayPlayer::from_reader(Cursor::new(wire(&records)));
    let mut app = App::new(Source::Replay(Box::new(player)));
    for message in app.source.poll(0.0) {
        app.handle(message);
    }
    for message in app.source.poll(0.0) {
        app.handle(message);
    }
    let Source::Replay(player) = &mut app.source else {
        unreachable!()
    };
    player.paused = true;
    player.advance_ticks(1.0);

    for message in app.source.poll(60.0) {
        app.handle(message);
    }

    assert_eq!(app.view.as_ref().unwrap().tick, 1);
    assert_eq!(app.shown_orders.len(), 1);
    assert_eq!(app.shown_orders[0].tick, 1);
    assert_eq!(app.feed.len(), 2);
}
