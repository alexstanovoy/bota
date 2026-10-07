//! Opt-in, headless verification of complete replay artifacts.

use std::cell::Cell;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bota_proto::{
    LEN_PREFIX, MAX_PAYLOAD_LEN, MatchStats, ReplayRecord, ServerMsg, Team, decode_payload,
};

use super::fixtures::message;
use crate::replay_play::{
    MAX_BYTES_PER_POLL, MAX_READS_PER_POLL, MAX_RECORDS_PER_POLL, REPLAY_BUFFER_CAPACITY,
    ReplayPlayer,
};

#[test]
#[ignore = "Requires BOTA_REPLAY_NEURAL and BOTA_REPLAY_TEACHER artifact paths."]
fn both_real_replays_stream_correctly_with_bounded_prefix() {
    for variable in ["BOTA_REPLAY_NEURAL", "BOTA_REPLAY_TEACHER"] {
        let path = std::env::var_os(variable).unwrap_or_else(|| panic!("set {variable}"));
        verify_replay(Path::new(&path));
    }
}

fn verify_replay(path: &Path) {
    let file = File::open(path).unwrap();
    let length = file.metadata().unwrap().len();
    assert!(length > 0);
    assert!(length <= 2 * 1024 * 1024 * 1024);
    let read = Rc::new(Cell::new((0, 0)));
    let start = Instant::now();
    let player = ReplayPlayer::from_reader(MeasuredFile {
        file,
        read: read.clone(),
    });
    let opened = start.elapsed();
    assert_eq!(read.get(), (0, 0), "construction must not read the file");
    let mut probe = Probe {
        player,
        read,
        reference: Reference {
            file: BufReader::new(File::open(path).unwrap()),
            payload: Vec::new(),
        },
        totals: Totals::new(),
        maxima: (0, 0, 0, 0),
    };
    for poll in 0..=length / LEN_PREFIX as u64 {
        let was_loading = probe.totals.snapshots == 0;
        probe.poll();
        if was_loading && probe.totals.snapshots > 0 {
            assert_eq!(
                probe.totals.snapshots, 1,
                "first snapshot is a render boundary"
            );
            assert!(
                poll < 64,
                "first snapshot must be within the bounded artifact prefix"
            );
            report_prefix(path, length, opened, start.elapsed(), poll + 1, &probe);
            probe.player.paused = true;
            probe.player.advance_ticks(f64::from(u32::MAX));
        }
        if probe.player.finished() {
            break;
        }
    }
    assert!(probe.player.finished(), "bounded scan must reach EOF");
    assert_eq!(probe.player.bytes_read(), length);
    assert_eq!(probe.read.get().0, length);
    assert!(
        probe.reference.next().unwrap().is_none(),
        "every record was played"
    );
    probe.totals.verify_complete();
    report_complete(path, start.elapsed(), &probe);
}

struct Probe {
    player: ReplayPlayer,
    read: Rc<Cell<(u64, u64)>>,
    reference: Reference,
    totals: Totals,
    maxima: (u64, u64, u64, usize),
}

impl Probe {
    fn poll(&mut self) {
        let before = (self.player.bytes_read(), self.read.get());
        let batch = self.player.poll(0.0);
        let logical = self.player.bytes_read() - before.0;
        let fetched = self.read.get().0 - before.1.0;
        let calls = self.read.get().1 - before.1.1;
        assert!(logical <= MAX_BYTES_PER_POLL as u64);
        assert!(fetched <= (MAX_BYTES_PER_POLL + REPLAY_BUFFER_CAPACITY) as u64);
        assert!(calls <= MAX_READS_PER_POLL as u64);
        assert!(batch.len() <= MAX_RECORDS_PER_POLL);
        self.maxima = (
            self.maxima.0.max(logical),
            self.maxima.1.max(fetched),
            self.maxima.2.max(calls),
            self.maxima.3.max(batch.len()),
        );
        for received in batch {
            let expected = self
                .reference
                .next()
                .unwrap()
                .expect("reference has a matching record");
            assert_eq!(
                received,
                message(expected),
                "record {}",
                self.totals.messages + 1
            );
            self.totals.observe(&received);
        }
        assert!(self.player.error().is_none(), "{:?}", self.player.error());
    }
}

fn report_prefix(
    path: &Path,
    length: u64,
    opened: Duration,
    first: Duration,
    polls: u64,
    probe: &Probe,
) {
    println!(
        "prefix path={} file_bytes={length} constructor_us={} first_snapshot_us={} polls={polls} records={} tick={:?} parser_bytes={} underlying_bytes={} read_calls={}",
        path.display(),
        opened.as_micros(),
        first.as_micros(),
        probe.totals.messages,
        probe.totals.first_tick,
        probe.player.bytes_read(),
        probe.read.get().0,
        probe.read.get().1
    );
}

fn report_complete(path: &Path, elapsed: Duration, probe: &Probe) {
    let totals = &probe.totals;
    let maxima = probe.maxima;
    println!(
        "complete path={} elapsed_ms={} records={} starts={} snapshots={} events={} orders={} first_tick={:?} last_tick={:?} winner={:?} duration={} parser_bytes={} max_poll_parser_bytes={} max_poll_underlying_bytes={} max_poll_reads={} max_batch={}",
        path.display(),
        elapsed.as_millis(),
        totals.messages,
        totals.starts,
        totals.snapshots,
        totals.events,
        totals.orders,
        totals.first_tick,
        totals.last_tick,
        totals.winner,
        totals.stats.as_ref().unwrap().duration,
        probe.player.bytes_read(),
        maxima.0,
        maxima.1,
        maxima.2,
        maxima.3
    );
}

struct MeasuredFile {
    file: File,
    read: Rc<Cell<(u64, u64)>>,
}

impl Read for MeasuredFile {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        let (bytes, calls) = self.read.get();
        self.read.set((bytes, calls + 1));
        let count = self.file.read(target)?;
        self.read.set((bytes + count as u64, calls + 1));
        Ok(count)
    }
}

struct Reference {
    file: BufReader<File>,
    payload: Vec<u8>,
}

impl Reference {
    fn next(&mut self) -> io::Result<Option<ReplayRecord>> {
        let mut prefix = [0; LEN_PREFIX];
        if self.file.read(&mut prefix[..1])? == 0 {
            return Ok(None);
        }
        self.file.read_exact(&mut prefix[1..])?;
        let length = u32::from_le_bytes(prefix) as usize;
        assert!(length > 0);
        assert!(length <= MAX_PAYLOAD_LEN);
        self.payload.resize(length, 0);
        self.file.read_exact(&mut self.payload)?;
        decode_payload(&self.payload)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

struct Totals {
    messages: usize,
    starts: usize,
    snapshots: usize,
    events: usize,
    orders: usize,
    first_tick: Option<u32>,
    last_tick: Option<u32>,
    winner: Option<Team>,
    stats: Option<MatchStats>,
}

impl Totals {
    fn new() -> Self {
        Self {
            messages: 0,
            starts: 0,
            snapshots: 0,
            events: 0,
            orders: 0,
            first_tick: None,
            last_tick: None,
            winner: None,
            stats: None,
        }
    }

    fn observe(&mut self, message: &ServerMsg) {
        assert!(self.winner.is_none(), "MatchOver is the final message");
        self.messages += 1;
        match message {
            ServerMsg::MatchStart { info } => {
                self.starts += 1;
                assert_eq!(info.tick_rate, 30);
            }
            ServerMsg::Snapshot { view } => {
                if let Some(previous) = self.last_tick {
                    assert_eq!(view.tick, previous + 1);
                }
                self.snapshots += 1;
                self.first_tick.get_or_insert(view.tick);
                self.last_tick = Some(view.tick);
            }
            ServerMsg::Events { tick, .. } => {
                self.events += 1;
                assert_eq!(Some(*tick), self.last_tick);
            }
            ServerMsg::Orders { .. } => self.orders += 1,
            ServerMsg::MatchOver { winner, stats } => {
                self.winner = Some(*winner);
                self.stats = Some(stats.clone());
            }
            _ => {}
        }
    }

    fn verify_complete(&self) {
        assert_eq!(self.starts, 1);
        assert!(self.snapshots > 0);
        assert!(self.events > 0);
        assert!(self.orders > 0);
        assert!(self.winner.is_some());
        let stats = self
            .stats
            .as_ref()
            .expect("complete replay includes MatchOver");
        assert_eq!(Some(stats.duration), self.last_tick);
    }
}
