use std::cell::Cell;
use std::io::{self, Cursor, Read};
use std::rc::Rc;

use bota_proto::{MAX_PAYLOAD_LEN, ReplayRecord, ServerMsg};

use super::fixtures::{counted, snapshot, wire};
use crate::replay_play::{MAX_READS_PER_POLL, ReplayPlayer};

fn assert_rejected(bytes: Vec<u8>, kind: io::ErrorKind, detail: &str) {
    let (reader, read, calls) = counted(bytes, 1);
    let mut player = ReplayPlayer::from_reader(reader);

    assert!(player.poll(0.0).is_empty());
    let error = player.error().expect("invalid input must be reported");
    assert_eq!(error.kind(), kind);
    assert_eq!(
        error.to_string(),
        format!("replay record 1 at byte 0: {detail}")
    );
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
fn empty_input_is_an_error_instead_of_clean_eof() {
    assert_rejected(
        Vec::new(),
        io::ErrorKind::InvalidData,
        "no replay records in the file",
    );
}

#[test]
fn zero_payload_length_is_rejected() {
    assert_rejected(
        vec![0; 4],
        io::ErrorKind::InvalidData,
        "empty payload length",
    );
}

#[test]
fn every_partial_length_prefix_reports_the_missing_bytes() {
    for length in 1..4 {
        assert_rejected(
            vec![1; length],
            io::ErrorKind::UnexpectedEof,
            &format!("truncated length prefix: expected 4 bytes, got {length}"),
        );
    }
}

#[test]
fn oversized_lengths_are_rejected_before_reading_a_payload() {
    for length in [MAX_PAYLOAD_LEN as u32 + 1, u32::MAX] {
        assert_rejected(
            length.to_le_bytes().to_vec(),
            io::ErrorKind::InvalidData,
            &format!("payload length {length} exceeds {MAX_PAYLOAD_LEN} bytes"),
        );
    }
}

#[test]
fn malformed_payload_is_not_treated_as_eof() {
    assert_rejected(
        vec![1, 0, 0, 0, 255],
        io::ErrorKind::InvalidData,
        "frame payload did not decode",
    );
}

#[test]
fn every_partial_payload_reports_the_missing_bytes() {
    let frame = wire(&[snapshot(0)]);
    let length = frame.len() - 4;
    for received in 0..length {
        assert_rejected(
            frame[..4 + received].to_vec(),
            io::ErrorKind::UnexpectedEof,
            &format!("truncated payload: expected {length} bytes, got {received}"),
        );
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
            "replay record 3 at byte {offset}: truncated length prefix: expected 4 bytes, got 2"
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
