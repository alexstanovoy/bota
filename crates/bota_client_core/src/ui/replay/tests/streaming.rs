use std::cell::Cell;
use std::io::{self, Cursor, Read};
use std::rc::Rc;

use bota_proto::{LobbySlot, MAX_PAYLOAD_LEN, PickPhase, ReplayRecord, ServerMsg, SlotId, Team};

use super::fixtures::{CountedRead, counted, frames, header, message, snapshot, wire};
use crate::ui::replay::{
    MAX_BYTES_PER_POLL, MAX_READS_PER_POLL, MAX_RECORDS_PER_POLL, REPLAY_BUFFER_CAPACITY,
    ReplayPlayer,
};

#[test]
fn first_snapshot_is_available_without_reading_the_replay_tail() {
    let prefix = wire(&[snapshot(0)]);
    let bytes = Rc::new(Cell::new(0));
    let reader = CountedRead {
        limit: prefix.len(),
        source: Cursor::new(prefix),
        bytes: bytes.clone(),
        calls: Rc::new(Cell::new(0)),
        fragment: usize::MAX,
    };

    let mut player = ReplayPlayer::from_reader(reader);
    assert_eq!(bytes.get(), 0, "construction does not read the file");
    let messages = player.poll(0.0);

    assert_eq!(messages.len(), 1);
    assert!(matches!(&messages[0], ServerMsg::Snapshot { view } if view.tick == 0));
    assert_eq!(bytes.get(), wire(&[snapshot(0)]).len());
}

#[test]
fn fragmented_io_preserves_every_message_for_each_fragment_size() {
    let records: Vec<_> = (0..20).map(snapshot).collect();
    for fragment in 1..=32 {
        let (reader, _, _) = counted(wire(&records), fragment);
        let mut player = ReplayPlayer::from_reader(reader);
        let mut received = player.poll(0.0);
        player.advance_ticks(20.0);
        for _ in 0..100 {
            received.extend(player.poll(0.0));
            if player.finished() {
                break;
            }
        }
        assert!(player.finished(), "fragment {fragment}");
        assert!(player.error().is_none());
        assert_eq!(
            received,
            records.iter().cloned().map(message).collect::<Vec<_>>()
        );
    }
}

/// A replay as a server writes it, one Zstandard stream, plays exactly as
/// its frames do, read in any fragments, chat lines among them.
#[test]
fn a_packed_replay_plays_as_its_frames_do() {
    use bota_proto::{Audience, Body, Heard, Line, Speaker};
    use ruzstd::encoding::{CompressionLevel, compress_to_vec};
    let mut records: Vec<_> = (0..20).map(snapshot).collect();
    records.insert(
        3,
        ReplayRecord::Heard(Heard {
            tick: Some(2),
            from: Speaker::Seat(SlotId(1)),
            line: Line {
                to: Audience::Team,
                body: Body::Text("мид 下路 한국어".to_string()),
            },
        }),
    );
    let packed = compress_to_vec(&wire(&records)[..], CompressionLevel::Fastest);
    for fragment in [1, 7, 4096] {
        let (reader, _, _) = counted(packed.clone(), fragment);
        let mut player = ReplayPlayer::from_reader(reader);
        let mut received = player.poll(0.0);
        player.advance_ticks(20.0);
        for _ in 0..200 {
            received.extend(player.poll(0.0));
            if player.finished() {
                break;
            }
        }
        assert!(player.error().is_none(), "{:?}", player.error());
        assert!(player.finished(), "fragment {fragment}");
        assert_eq!(
            received,
            records.iter().cloned().map(message).collect::<Vec<_>>()
        );
    }
}

/// A replay of `remaining` bytes: this build's header, then one frame over
/// and over.
struct RepeatedRead {
    head: Cursor<Vec<u8>>,
    frame: Vec<u8>,
    remaining: u64,
    position: usize,
    bytes: Rc<Cell<usize>>,
}

impl RepeatedRead {
    fn new(frame: Vec<u8>, remaining: u64, bytes: Rc<Cell<usize>>) -> RepeatedRead {
        RepeatedRead {
            head: Cursor::new(header()),
            frame,
            remaining,
            position: 0,
            bytes,
        }
    }
}

impl Read for RepeatedRead {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        let length = target.len().min(self.remaining as usize);
        let from_head = self.head.read(&mut target[..length])?;
        for byte in &mut target[from_head..length] {
            *byte = self.frame[self.position];
            self.position = (self.position + 1) % self.frame.len();
        }
        self.remaining -= length as u64;
        self.bytes.set(self.bytes.get() + length);
        Ok(length)
    }
}

#[test]
fn virtual_gigabyte_replays_have_constant_startup_and_per_poll_cost() {
    let frame = frames(&[snapshot(0)]);
    let head = header().len() as u64;
    for length in [1024 * 1024, 1024 * 1024 * 1024] {
        let bytes = Rc::new(Cell::new(0));
        let reader = RepeatedRead::new(frame.clone(), length, bytes.clone());
        let mut player = ReplayPlayer::from_reader(reader);
        assert_eq!(bytes.get(), 0);
        assert_eq!(player.poll(60.0), vec![message(snapshot(0))]);
        assert_eq!(player.bytes_read(), head + frame.len() as u64);
        assert_eq!(bytes.get(), REPLAY_BUFFER_CAPACITY);
        let mut received = 1;
        for _ in 0..20 {
            let before = player.bytes_read();
            let batch = player.poll(60.0);
            assert!(!batch.is_empty());
            assert!(batch.len() <= MAX_RECORDS_PER_POLL);
            received += batch.len();
            assert!(player.bytes_read() - before <= MAX_BYTES_PER_POLL as u64);
            assert_eq!(
                (player.bytes_read() - head) / frame.len() as u64,
                received as u64
            );
            assert!(!player.finished());
            assert!(player.error().is_none());
        }
    }
}

fn maximum_payload_record() -> ReplayRecord {
    let mut record = ReplayRecord::Msg(ServerMsg::LobbyState {
        slots: vec![LobbySlot {
            slot: SlotId(0),
            team: Team::Radiant,
            name: "x".repeat(MAX_PAYLOAD_LEN - 64),
            role: None,
            hero: None,
            ready: false,
        }],
        same_heroes: true,
        pick: PickPhase::Waiting,
        heroes: Vec::new(),
        catalogue: bota_core::Catalogue::FULL.prefix(),
    });
    let length = frames(&[record.clone()]).len() - 4;
    let ReplayRecord::Msg(ServerMsg::LobbyState { slots, .. }) = &mut record else {
        unreachable!()
    };
    slots[0]
        .name
        .extend(std::iter::repeat_n('x', MAX_PAYLOAD_LEN - length));
    assert_eq!(frames(&[record.clone()]).len(), MAX_PAYLOAD_LEN + 4);
    record
}

#[test]
fn maximum_payload_spans_polls_without_exceeding_byte_or_read_budgets() {
    let record = maximum_payload_record();
    let (reader, bytes, calls) = counted(wire(&[record.clone(), snapshot(0)]), usize::MAX);
    let mut player = ReplayPlayer::from_reader(reader);
    let mut received = Vec::new();
    for _ in 0..32 {
        let before = (player.bytes_read(), bytes.get(), calls.get());
        received.extend(player.poll(60.0));
        assert!(player.bytes_read() - before.0 <= MAX_BYTES_PER_POLL as u64);
        assert!(bytes.get() - before.1 <= MAX_BYTES_PER_POLL + REPLAY_BUFFER_CAPACITY);
        assert!(calls.get() - before.2 <= MAX_READS_PER_POLL);
        assert!(player.error().is_none());
        if received.len() == 2 {
            break;
        }
    }
    assert_eq!(received, vec![message(record), message(snapshot(0))]);
    assert_eq!(player.status(), "playing");
}

#[test]
fn read_call_budget_yields_with_a_partial_payload_and_preserves_loading_clock() {
    let mut record = snapshot(0);
    let ReplayRecord::Msg(ServerMsg::Snapshot { view }) = &mut record else {
        unreachable!()
    };
    view.felled_trees = vec![1; MAX_READS_PER_POLL * 2];
    let (reader, _, calls) = counted(wire(&[record.clone(), snapshot(1)]), 1);
    let mut player = ReplayPlayer::from_reader(reader);
    assert!(player.poll(60.0).is_empty());
    assert_eq!(calls.get(), MAX_READS_PER_POLL);
    assert_eq!(player.status(), "loading");
    let mut received = Vec::new();
    for _ in 0..8 {
        let before = calls.get();
        received.extend(player.poll(60.0));
        assert!(calls.get() - before <= MAX_READS_PER_POLL);
        if !received.is_empty() {
            break;
        }
    }
    assert_eq!(received, vec![message(record)]);
    assert!(player.poll(60.0).is_empty());
    assert_eq!(player.poll(1.0 / 30.0), vec![message(snapshot(1))]);
}

#[test]
fn queued_seek_completes_under_backpressure_without_adding_elapsed_time() {
    let records: Vec<_> = (0..300).map(snapshot).collect();
    let mut player = ReplayPlayer::from_reader(Cursor::new(wire(&records)));
    player.poll(0.0);
    player.advance_ticks(150.0);
    let mut received = Vec::new();
    for _ in 0..4 {
        received.extend(player.poll(60.0));
        if player.status() != "catching up" {
            break;
        }
    }
    assert_eq!(
        received,
        (1..=150)
            .map(|tick| message(snapshot(tick)))
            .collect::<Vec<_>>()
    );
    assert_eq!(player.poll(1.0 / 30.0), vec![message(snapshot(151))]);
}

#[test]
fn orders_before_any_snapshot_flow_out_in_bounded_batches() {
    let record = ReplayRecord::Orders {
        tick: 0,
        orders: Vec::new(),
    };
    let reader = RepeatedRead::new(frames(&[record]), 1024 * 1024 * 1024, Rc::new(Cell::new(0)));
    let mut player = ReplayPlayer::from_reader(reader);
    for _ in 0..100 {
        let batch = player.poll(0.0);
        assert!(!batch.is_empty());
        assert!(batch.len() <= MAX_RECORDS_PER_POLL);
        assert!(
            batch
                .iter()
                .all(|message| matches!(message, ServerMsg::Orders { .. }))
        );
    }
    assert!(player.error().is_none());
}
