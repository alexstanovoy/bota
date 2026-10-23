use std::cell::Cell;
use std::io::{self, Cursor, Read};
use std::rc::Rc;

use bota_proto::{
    MAX_PAYLOAD_LEN, ReplayHeader, ReplayRecord, ServerMsg, WIRE_FINGERPRINT, fingerprint_to_hex,
};

use super::fixtures::{counted, frames, header, snapshot, wire};
use crate::ui::replay::{MAX_READS_PER_POLL, ReplayPlayer};

/// The refusal of a first record that is no header this build reads.
const NO_HEADER: &str =
    "no replay header this build reads: not a bota replay, or one recorded before replays had one";

fn assert_rejected(bytes: Vec<u8>, kind: io::ErrorKind, detail: &str) {
    assert_refused_at(bytes, kind, &format!("replay record 1 at byte 0: {detail}"));
}

fn assert_refused_at(bytes: Vec<u8>, kind: io::ErrorKind, message: &str) {
    let (reader, read, calls) = counted(bytes, 1);
    let mut player = ReplayPlayer::from_reader(reader);

    assert!(player.poll(0.0).is_empty());
    let error = player.error().expect("invalid input must be reported");
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), message);
    assert!(!player.finished());
    assert!(player.paused);
    assert_eq!(player.status(), "error");
    let before = (read.get(), calls.get());
    player.advance_ticks(1.0);
    assert!(player.poll(1.0).is_empty());
    assert_eq!(
        (read.get(), calls.get()),
        before,
        "failed streams are terminal"
    );
}

#[test]
fn every_invalid_framing_is_rejected_with_its_reason() {
    use io::ErrorKind::{InvalidData, UnexpectedEof};
    let mut cases = vec![
        (
            Vec::new(),
            InvalidData,
            "no replay records in the file".to_string(),
        ),
        (vec![0; 4], InvalidData, "empty payload length".to_string()),
        (vec![1, 0, 0, 0, 255], InvalidData, NO_HEADER.to_string()),
    ];
    for length in 1..4 {
        let detail = format!("truncated length prefix: expected 4 bytes, got {length}");
        cases.push((vec![1; length], UnexpectedEof, detail));
    }
    for length in [MAX_PAYLOAD_LEN as u32 + 1, u32::MAX] {
        let detail = format!("payload length {length} exceeds {MAX_PAYLOAD_LEN} bytes");
        cases.push((length.to_le_bytes().to_vec(), InvalidData, detail));
    }
    let frame = frames(&[snapshot(0)]);
    let length = frame.len() - 4;
    for received in 0..length {
        let detail = format!("truncated payload: expected {length} bytes, got {received}");
        cases.push((frame[..4 + received].to_vec(), UnexpectedEof, detail));
    }
    for (bytes, kind, detail) in cases {
        assert_rejected(bytes, kind, &detail);
    }
}

/// A replay is played only behind this build's header: one of another wire
/// is refused up front with both fingerprints named, before any message.
#[test]
fn a_replay_without_this_builds_header_is_refused_before_any_message() {
    use io::ErrorKind::InvalidData;
    let other_wire = ReplayRecord::Header(ReplayHeader {
        wire_fingerprint: !WIRE_FINGERPRINT,
        ..ReplayHeader::CURRENT
    });
    let not_a_replay = ReplayRecord::Header(ReplayHeader {
        magic: *b"bota.brq",
        ..ReplayHeader::CURRENT
    });
    let second = ReplayRecord::Header(ReplayHeader::CURRENT);
    let cases = [
        (frames(&[snapshot(0)]), 1, 0, NO_HEADER.to_string()),
        (
            frames(&[other_wire, snapshot(0)]),
            1,
            0,
            format!(
                "recorded with wire fingerprint {}; this build plays {}",
                fingerprint_to_hex(!WIRE_FINGERPRINT),
                fingerprint_to_hex(WIRE_FINGERPRINT)
            ),
        ),
        (
            frames(&[not_a_replay, snapshot(0)]),
            1,
            0,
            "not a bota replay".to_string(),
        ),
        (
            wire(&[second, snapshot(0)]),
            2,
            header().len(),
            "a second replay header".to_string(),
        ),
    ];
    for (bytes, record, byte, detail) in cases {
        let message = format!("replay record {record} at byte {byte}: {detail}");
        assert_refused_at(bytes, InvalidData, &message);
    }
}

#[test]
fn deferred_failure_preserves_prior_messages_and_reports_record_and_offset() {
    let event = ReplayRecord::Msg(ServerMsg::Events {
        tick: 0,
        events: Vec::new(),
    });
    let mut bytes = wire(&[snapshot(0), event.clone()]);
    let offset = bytes.len();
    bytes.extend_from_slice(&[1, 0]);
    let mut player = ReplayPlayer::from_reader(Cursor::new(bytes));
    assert_eq!(player.poll(0.0).len(), 1);
    assert!(player.error().is_none());

    let messages = player.poll(0.0);

    assert_eq!(messages, vec![super::fixtures::message(event)]);
    let error = player.error().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(
        error.to_string(),
        format!(
            "replay record 4 at byte {offset}: truncated length prefix: expected 4 bytes, got 2"
        )
    );
    assert!(!player.finished());
}

#[test]
fn clean_eof_is_terminal_and_is_not_an_error() {
    let (reader, bytes, calls) = counted(wire(&[snapshot(0)]), 1);
    let mut player = ReplayPlayer::from_reader(reader);
    assert_eq!(player.poll(0.0).len(), 1);
    assert!(!player.finished(), "EOF is checked on a subsequent poll");

    assert!(player.poll(0.0).is_empty());

    assert!(player.finished());
    assert!(player.error().is_none());
    assert_eq!(player.status(), "finished");
    let before = (bytes.get(), calls.get());
    assert!(player.poll(60.0).is_empty());
    assert_eq!((bytes.get(), calls.get()), before);
}

struct InterruptedRead {
    calls: Rc<Cell<usize>>,
    remaining: usize,
    source: Cursor<Vec<u8>>,
}

impl Read for InterruptedRead {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        self.calls.set(self.calls.get() + 1);
        if self.remaining > 0 {
            self.remaining -= 1;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        self.source.read(target)
    }
}

#[test]
fn repeated_interruptions_yield_at_the_read_budget_and_resume() {
    let calls = Rc::new(Cell::new(0));
    let reader = InterruptedRead {
        calls: calls.clone(),
        remaining: MAX_READS_PER_POLL,
        source: Cursor::new(wire(&[snapshot(0)])),
    };
    let mut player = ReplayPlayer::from_reader(reader);

    assert!(player.poll(60.0).is_empty());

    assert_eq!(calls.get(), MAX_READS_PER_POLL);
    assert_eq!(player.bytes_read(), 0);
    assert!(player.error().is_none());
    assert_eq!(player.status(), "loading");
    assert_eq!(player.poll(60.0).len(), 1);
}

#[test]
fn reader_failure_keeps_the_io_kind_and_context() {
    let (mut reader, _, _) = counted(vec![1, 0], 1);
    reader.limit = 2;
    let mut player = ReplayPlayer::from_reader(reader);

    assert!(player.poll(0.0).is_empty());

    let error = player.error().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(
        error.to_string(),
        "replay record 1 at byte 0: read crossed the permitted replay prefix"
    );
}
