//! Streaming a `.brp` file on its recorded clock.

use std::fs::File;
use std::io::{self, BufReader, Cursor, Read};
use std::path::Path;

use bota_core::WIRE_FINGERPRINT;
use bota_proto::{
    LEN_PREFIX, MAX_PAYLOAD_LEN, ReplayHeader, ReplayRecord, ServerMsg, decode_payload,
    fingerprint_to_hex,
};

use crate::replay_file::{UnpackedReplay, ZSTD_MAGIC};

/// Maximum decoded records released or inspected in one poll.
pub const MAX_RECORDS_PER_POLL: usize = 64;
/// Maximum framing and payload bytes consumed in one poll.
pub const MAX_BYTES_PER_POLL: usize = 256 * 1024;
/// Maximum reader calls in one poll, including interrupted calls.
pub const MAX_READS_PER_POLL: usize = 128;
/// File read-ahead capacity in bytes.
pub const REPLAY_BUFFER_CAPACITY: usize = 8 * 1024;

const _: () = assert!(REPLAY_BUFFER_CAPACITY <= MAX_BYTES_PER_POLL);
const _: () = assert!(MAX_BYTES_PER_POLL <= MAX_PAYLOAD_LEN);

struct ReadBudget {
    bytes: usize,
    calls: usize,
}

enum RecordPoll {
    Pending,
    Record(Box<ReplayRecord>),
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
    /// Whether the file turned out to be a Zstandard stream, now read
    /// unpacked. A stream inside the stream is not unpacked again.
    unpacked: bool,
}

/// Feeds bounded batches of recorded messages out on the recorded clock.
///
/// Snapshots and order records wait for their ticks. Messages between them
/// retain file order. Order records become [`ServerMsg::Orders`], line records
/// [`ServerMsg::Heard`].
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
    /// Whether elapsed time is kept off the clock; records already due still
    /// come out.
    pub paused: bool,
    /// Positive, finite clock multiplier: 1.0 is the recorded pace.
    pub speed: f32,
}

impl ReplayPlayer {
    /// Opens a replay without reading records. Read errors appear in [`Self::error`].
    pub fn load(path: &Path) -> io::Result<Self> {
        Ok(Self::from_reader(File::open(path)?))
    }

    /// Creates a player without consuming any bytes from the reader, which
    /// holds the frames packed as a server writes them
    /// ([`UnpackedReplay`]), or as they are.
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
                unpacked: false,
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
    #[cfg(test)]
    pub(crate) fn bytes_read(&self) -> u64 {
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
            let record = match self.pending.take() {
                Some(record) => record,
                None => match self.stream.poll_record(&mut budget) {
                    Ok(RecordPoll::Record(record)) => *record,
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
                },
            };
            let tick = match &record {
                ReplayRecord::Orders { tick, .. } => Some(*tick),
                ReplayRecord::Msg(ServerMsg::Snapshot { view }) => Some(view.tick),
                ReplayRecord::Heard(heard) => heard.tick,
                _ => None,
            };
            if self.started && tick.is_some_and(|tick| f64::from(tick) > self.clock_ticks) {
                self.pending = Some(record);
                return due;
            }
            let message = match record {
                ReplayRecord::Orders { tick, orders } => ServerMsg::Orders { tick, orders },
                ReplayRecord::Msg(message) => message,
                ReplayRecord::Heard(heard) => ServerMsg::Heard(heard),
                // The stream takes the one header in before any record comes out.
                ReplayRecord::Header(_) => continue,
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
        self.catching_up = true;
        due
    }
}

/// Why a replay is refused up front. It is the inner error of the
/// [`io::Error`] the player reports, so a front end can tell the cases
/// apart with `downcast_ref`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayRefusal {
    /// The first record is a header of something else.
    NotBota,
    /// Recorded with another wire fingerprint than this build plays.
    OtherVersion { recorded: u64, plays: u64 },
    /// The first record is no header this build reads.
    NoHeader,
    /// A header where only the first record may be one.
    SecondHeader,
}

impl std::fmt::Display for ReplayRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotBota => f.write_str("not a bota replay"),
            Self::OtherVersion { recorded, plays } => write!(
                f,
                "recorded with wire fingerprint {}; this build plays {}",
                fingerprint_to_hex(*recorded),
                fingerprint_to_hex(*plays)
            ),
            Self::NoHeader => f.write_str(
                "no replay header this build reads: not a bota replay, or one recorded before \
                 replays had one",
            ),
            Self::SecondHeader => f.write_str("a second replay header"),
        }
    }
}

impl std::error::Error for ReplayRefusal {}

/// Why a record cannot stand where it does: the first must be this build's
/// header, and no other may be a header.
fn header_refusal(record: &ReplayRecord, first: bool) -> Option<ReplayRefusal> {
    match (record, first) {
        (ReplayRecord::Header(header), true) if *header == ReplayHeader::new(WIRE_FINGERPRINT) => {
            None
        }
        (ReplayRecord::Header(header), true) if header.magic != ReplayHeader::MAGIC => {
            Some(ReplayRefusal::NotBota)
        }
        (ReplayRecord::Header(header), true) => Some(ReplayRefusal::OtherVersion {
            recorded: header.wire_fingerprint,
            plays: WIRE_FINGERPRINT,
        }),
        (_, true) => Some(ReplayRefusal::NoHeader),
        (ReplayRecord::Header(_), false) => Some(ReplayRefusal::SecondHeader),
        (_, false) => None,
    }
}

impl ReplayStream {
    fn poll_record(&mut self, budget: &mut ReadBudget) -> io::Result<RecordPoll> {
        for _ in 0..=MAX_READS_PER_POLL {
            if self.prefix_read == LEN_PREFIX {
                if self.records == 0 && !self.unpacked && self.prefix == ZSTD_MAGIC {
                    self.unpack();
                    continue;
                }
                if self.payload.is_empty() {
                    self.prepare_payload()?;
                }
                if self.payload_read == self.payload.len() {
                    let first = self.records == 0;
                    let record = decode_payload(&self.payload).map_err(|error| {
                        if first {
                            io::Error::new(io::ErrorKind::InvalidData, ReplayRefusal::NoHeader)
                        } else {
                            self.failure(io::ErrorKind::InvalidData, error)
                        }
                    })?;
                    if let Some(refusal) = header_refusal(&record, first) {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, refusal));
                    }
                    self.prefix_read = 0;
                    self.payload.clear();
                    self.payload_read = 0;
                    self.records += 1;
                    // The header says what the file is; the match follows it.
                    if !first {
                        return Ok(RecordPoll::Record(Box::new(record)));
                    }
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

    /// Reads the rest of the file, the magic just read and what was read
    /// ahead of it put back in front, as the stream it is.
    fn unpack(&mut self) {
        let empty: Box<dyn Read> = Box::new(io::empty());
        let packed = std::mem::replace(&mut self.reader, BufReader::new(empty));
        let ahead = [&ZSTD_MAGIC[..], packed.buffer()].concat();
        let file = Cursor::new(ahead).chain(packed.into_inner());
        self.reader =
            BufReader::with_capacity(REPLAY_BUFFER_CAPACITY, Box::new(UnpackedReplay::new(file)));
        self.unpacked = true;
        self.prefix_read = 0;
        self.offset = 0;
    }

    fn prepare_payload(&mut self) -> io::Result<()> {
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
        Ok(())
    }

    fn read_chunk(&mut self, budget: &mut ReadBudget) -> io::Result<usize> {
        let (target, progress) = if self.prefix_read < LEN_PREFIX {
            (&mut self.prefix[..], &mut self.prefix_read)
        } else {
            (&mut self.payload[..], &mut self.payload_read)
        };
        let length = (target.len() - *progress).min(budget.bytes);
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

#[cfg(test)]
mod tests;
