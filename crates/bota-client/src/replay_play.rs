//! Streaming a `.brp` file on its recorded clock.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use bota_proto::{LEN_PREFIX, MAX_PAYLOAD_LEN, ReplayRecord, ServerMsg, decode_payload};

/// Maximum decoded records released or inspected in one poll.
pub const MAX_RECORDS_PER_POLL: usize = 64;
/// Maximum framing and payload bytes consumed in one poll.
pub const MAX_BYTES_PER_POLL: usize = 256 * 1024;
/// Maximum reader calls in one poll, including interrupted calls.
pub const MAX_READS_PER_POLL: usize = 128;
/// File read-ahead capacity in bytes.
pub const REPLAY_BUFFER_CAPACITY: usize = 8 * 1024;

const _: () = assert!(MAX_RECORDS_PER_POLL > 0);
const _: () = assert!(MAX_READS_PER_POLL > 0);
const _: () = assert!(REPLAY_BUFFER_CAPACITY <= MAX_BYTES_PER_POLL);
const _: () = assert!(MAX_BYTES_PER_POLL <= MAX_PAYLOAD_LEN);

struct ReadBudget {
    bytes: usize,
    calls: usize,
}

enum RecordPoll {
    Pending,
    Record(ReplayRecord),
    Eof,
}

struct ReplayStream {
    reader: BufReader<Box<dyn Read>>,
    prefix: [u8; LEN_PREFIX],
    prefix_read: usize,
    payload: Vec<u8>,
    payload_read: usize,
    offset: u64,
    records: u64,
}

/// Feeds bounded batches of recorded messages out on the recorded clock.
///
/// Snapshots and order records wait for their ticks. Messages between them
/// retain file order. Order records become [`ServerMsg::Orders`].
pub struct ReplayPlayer {
    stream: ReplayStream,
    pending: Option<ReplayRecord>,
    error: Option<io::Error>,
    eof: bool,
    started: bool,
    discard_dt: bool,
    catching_up: bool,
    clock_ticks: f64,
    tick_rate: f64,
    /// Whether elapsed time advances the clock; queued work still completes.
    pub paused: bool,
    /// Positive, finite clock multiplier: 1.0 is the recorded pace.
    pub speed: f32,
}

impl ReplayPlayer {
    /// Opens a replay without reading records. Read errors appear in [`Self::error`].
    pub fn load(path: &Path) -> io::Result<Self> {
        Ok(Self::from_reader(File::open(path)?))
    }

    /// Creates a player without consuming any bytes from the reader.
    pub fn from_reader(reader: impl Read + 'static) -> Self {
        Self {
            stream: ReplayStream {
                reader: BufReader::with_capacity(REPLAY_BUFFER_CAPACITY, Box::new(reader)),
                prefix: [0; LEN_PREFIX],
                prefix_read: 0,
                payload: Vec::new(),
                payload_read: 0,
                offset: 0,
                records: 0,
            },
            pending: None,
            error: None,
            eof: false,
            started: false,
            discard_dt: false,
            catching_up: false,
            clock_ticks: 0.0,
            tick_rate: 30.0,
            paused: false,
            speed: 1.0,
        }
    }

    /// Advances elapsed seconds and performs at most one batch of replay work.
    ///
    /// Call once per GUI frame. Loading, the first snapshot's render interval,
    /// and budget-limited catch-up do not add elapsed time to the clock.
    /// Already decoded messages are returned even if a later record fails.
    pub fn poll(&mut self, dt: f32) -> Vec<ServerMsg> {
        assert!(dt.is_finite());
        assert!(dt >= 0.0);
        assert!(self.speed.is_finite());
        assert!(self.speed > 0.0);
        if self.eof || self.error.is_some() {
            return Vec::new();
        }
        if self.started && !self.discard_dt && !self.paused && !self.catching_up {
            self.advance_ticks(f64::from(dt) * f64::from(self.speed) * self.tick_rate);
        }
        self.discard_dt = false;
        self.drain_due()
    }

    /// Queues a forward clock jump in ticks, paused or not; performs no I/O.
    pub fn advance_ticks(&mut self, ticks: f64) {
        assert!(ticks.is_finite());
        assert!(ticks >= 0.0);
        self.clock_ticks = (self.clock_ticks + ticks).min(f64::from(u32::MAX));
    }

    /// Whether clean EOF was reached after all records were played.
    pub fn finished(&self) -> bool {
        self.eof
    }

    /// Terminal framing, decoding or I/O failure; absent on clean EOF.
    pub fn error(&self) -> Option<&io::Error> {
        self.error.as_ref()
    }

    /// Bytes consumed by the parser, excluding file read-ahead.
    pub fn bytes_read(&self) -> u64 {
        self.stream.offset
    }

    /// Playback state shown by the replay UI.
    pub fn status(&self) -> &'static str {
        if self.error.is_some() {
            "error"
        } else if self.finished() {
            "finished"
        } else if !self.started {
            "loading"
        } else if self.catching_up {
            "catching up"
        } else if self.paused {
            "paused"
        } else {
            "playing"
        }
    }

    fn drain_due(&mut self) -> Vec<ServerMsg> {
        let mut due = Vec::new();
        let mut budget = ReadBudget {
            bytes: MAX_BYTES_PER_POLL,
            calls: MAX_READS_PER_POLL,
        };
        self.catching_up = false;
        for _ in 0..MAX_RECORDS_PER_POLL {
            if self.pending.is_none() {
                match self.stream.poll_record(&mut budget) {
                    Ok(RecordPoll::Record(record)) => self.pending = Some(record),
                    Ok(RecordPoll::Pending) => {
                        self.catching_up = true;
                        return due;
                    }
                    Ok(RecordPoll::Eof) => {
                        self.eof = true;
                        return due;
                    }
                    Err(error) => {
                        self.paused = true;
                        self.error = Some(error);
                        return due;
                    }
                }
            }
            let record = self.pending.as_ref().expect("one record was read");
            let tick = match record {
                ReplayRecord::Orders { tick, .. } => Some(*tick),
                ReplayRecord::Msg(ServerMsg::Snapshot { view }) => Some(view.tick),
                _ => None,
            };
            if self.started && tick.is_some_and(|tick| f64::from(tick) > self.clock_ticks) {
                return due;
            }
            let record = self
                .pending
                .take()
                .expect("the pending record was inspected");
            let message = match record {
                ReplayRecord::Orders { tick, orders } => ServerMsg::Orders { tick, orders },
                ReplayRecord::Msg(message) => message,
            };
            if let ServerMsg::MatchStart { info } = &message {
                self.tick_rate = f64::from(info.tick_rate.max(1));
            }
            let first_snapshot = !self.started && matches!(message, ServerMsg::Snapshot { .. });
            if first_snapshot {
                self.started = true;
                self.discard_dt = true;
                self.advance_ticks(f64::from(tick.expect("snapshots carry a tick")));
            }
            due.push(message);
            if first_snapshot {
                return due;
            }
        }
        assert!(due.len() <= MAX_RECORDS_PER_POLL);
        self.catching_up = true;
        due
    }
}

impl ReplayStream {
    fn poll_record(&mut self, budget: &mut ReadBudget) -> io::Result<RecordPoll> {
        assert!(self.prefix_read <= LEN_PREFIX);
        assert!(self.payload_read <= self.payload.len());
        for _ in 0..=MAX_READS_PER_POLL {
            if self.prefix_read == LEN_PREFIX {
                if self.payload.is_empty() {
                    self.prepare_payload()?;
                }
                if self.payload_read == self.payload.len() {
                    let record = decode_payload(&self.payload)
                        .map_err(|error| self.failure(io::ErrorKind::InvalidData, error))?;
                    self.prefix_read = 0;
                    self.payload.clear();
                    self.payload_read = 0;
                    self.records += 1;
                    return Ok(RecordPoll::Record(record));
                }
            }
            if budget.bytes == 0 || budget.calls == 0 {
                return Ok(RecordPoll::Pending);
            }
            match self.read_chunk(budget) {
                Ok(0) => return self.end_of_input(),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(self.failure(error.kind(), error)),
            }
        }
        Ok(RecordPoll::Pending)
    }

    fn prepare_payload(&mut self) -> io::Result<()> {
        assert_eq!(self.prefix_read, LEN_PREFIX);
        assert!(self.payload.is_empty());
        let length = u32::from_le_bytes(self.prefix) as usize;
        if length == 0 {
            return Err(self.failure(io::ErrorKind::InvalidData, "empty payload length"));
        }
        if length > MAX_PAYLOAD_LEN {
            return Err(self.failure(
                io::ErrorKind::InvalidData,
                format!("payload length {length} exceeds {MAX_PAYLOAD_LEN} bytes"),
            ));
        }
        self.payload.reserve_exact(length);
        self.payload.resize(length, 0);
        assert!(self.payload.capacity() <= MAX_PAYLOAD_LEN);
        Ok(())
    }

    fn read_chunk(&mut self, budget: &mut ReadBudget) -> io::Result<usize> {
        assert!(budget.bytes > 0);
        assert!(budget.calls > 0);
        let (target, progress) = if self.prefix_read < LEN_PREFIX {
            (&mut self.prefix[..], &mut self.prefix_read)
        } else {
            (&mut self.payload[..], &mut self.payload_read)
        };
        let length = (target.len() - *progress).min(budget.bytes);
        assert!(length > 0);
        budget.calls -= 1;
        let count = self
            .reader
            .read(&mut target[*progress..*progress + length])?;
        budget.bytes -= count;
        *progress += count;
        self.offset += count as u64;
        Ok(count)
    }

    fn end_of_input(&self) -> io::Result<RecordPoll> {
        if self.prefix_read == 0 {
            if self.records == 0 {
                return Err(
                    self.failure(io::ErrorKind::InvalidData, "no replay records in the file")
                );
            }
            return Ok(RecordPoll::Eof);
        }
        let detail = if self.prefix_read < LEN_PREFIX {
            format!(
                "truncated length prefix: expected {LEN_PREFIX} bytes, got {}",
                self.prefix_read
            )
        } else {
            format!(
                "truncated payload: expected {} bytes, got {}",
                self.payload.len(),
                self.payload_read
            )
        };
        Err(self.failure(io::ErrorKind::UnexpectedEof, detail))
    }

    fn failure(&self, kind: io::ErrorKind, detail: impl std::fmt::Display) -> io::Error {
        let start = self.offset - (self.prefix_read + self.payload_read) as u64;
        io::Error::new(
            kind,
            format!(
                "replay record {} at byte {start}: {detail}",
                self.records + 1
            ),
        )
    }
}
