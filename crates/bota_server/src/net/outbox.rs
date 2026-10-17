//! The outgoing side of one connection.

use std::collections::VecDeque;
use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex};

use bota_proto::{
    EventKind, LEN_PREFIX, ServerMsg, SkippedTick, decode_payload, encode_frame,
    encode_frame_to_vec,
};

/// Frames waiting to leave on one connection.
///
/// The simulation thread pushes and never blocks; a writer thread drains onto
/// the socket at whatever pace the peer accepts. A tick, its snapshot and its
/// events, is coalescible: queueing a new one replaces any older tick still
/// waiting, whose events then ride along in the new snapshot's
/// [`skipped_ticks`](bota_proto::WorldView::skipped_ticks), so a slow consumer
/// skips states but no events. Everything else is kept in order and never
/// dropped; a queue that overflows, or a peer that skips more than
/// [`MAX_SKIPPED_TICKS`] in a row, closes the connection instead.
pub struct Outbox {
    state: Mutex<OutboxState>,
    wake: Condvar,
}

struct OutboxState {
    queue: VecDeque<Queued>,
    closed: bool,
}

/// One entry of the queue.
enum Queued {
    /// A frame that must arrive as it is.
    Frame(Vec<u8>),
    /// A tick: its snapshot frame, its events and the ticks it replaced.
    Tick(Box<QueuedTick>),
}

struct QueuedTick {
    /// The encoded snapshot of the tick, without skipped ticks.
    snapshot: Vec<u8>,
    tick: u32,
    events: Vec<EventKind>,
    /// Ticks this one replaced, oldest first.
    skipped: Vec<SkippedTick>,
}

/// Queued entries past this many close the connection as hopeless.
const QUEUE_LIMIT: usize = 1024;

/// Ticks one snapshot may carry past before the connection is closed as
/// hopeless: thirty seconds at 30 ticks a second.
pub const MAX_SKIPPED_TICKS: usize = 900;

impl Outbox {
    /// An empty, open outbox.
    pub fn new() -> Arc<Outbox> {
        Arc::new(Outbox {
            state: Mutex::new(OutboxState {
                queue: VecDeque::new(),
                closed: false,
            }),
            wake: Condvar::new(),
        })
    }

    /// Queues a frame that must arrive.
    pub fn push(&self, bytes: Vec<u8>) {
        self.enqueue(Queued::Frame(bytes));
    }

    /// Queues a tick, its encoded snapshot and its events, replacing any tick
    /// still waiting.
    pub fn push_tick(&self, snapshot: Vec<u8>, tick: u32, events: Vec<EventKind>) {
        self.enqueue(Queued::Tick(Box::new(QueuedTick {
            snapshot,
            tick,
            events,
            skipped: Vec::new(),
        })));
    }

    fn enqueue(&self, mut entry: Queued) {
        let mut state = self.state.lock().expect("outbox lock");
        if state.closed {
            return;
        }
        if let Queued::Tick(newer) = &mut entry
            && let Some(at) = state
                .queue
                .iter()
                .position(|queued| matches!(queued, Queued::Tick(_)))
        {
            let Some(Queued::Tick(older)) = state.queue.remove(at) else {
                unreachable!("the position holds a tick");
            };
            let older = *older;
            assert!(older.tick < newer.tick, "ticks are queued in order");
            newer.skipped = older.skipped;
            newer.skipped.push(SkippedTick {
                tick: older.tick,
                events: older.events,
            });
            if newer.skipped.len() > MAX_SKIPPED_TICKS {
                state.closed = true;
            }
        }
        state.queue.push_back(entry);
        if state.queue.len() > QUEUE_LIMIT {
            state.closed = true;
        }
        drop(state);
        self.wake.notify_one();
    }

    /// Lets the writer drain what is queued, then stop.
    pub fn close(&self) {
        self.state.lock().expect("outbox lock").closed = true;
        self.wake.notify_one();
    }

    /// The writer thread body: drains frames onto the socket until the outbox
    /// closes and empties, or the socket fails.
    pub fn run_writer(self: &Arc<Outbox>, mut stream: TcpStream) {
        loop {
            let entry = {
                let state = self.state.lock().expect("outbox lock");
                self.wake
                    .wait_while(state, |state| state.queue.is_empty() && !state.closed)
                    .expect("outbox lock")
                    .queue
                    .pop_front()
            };
            let Some(entry) = entry else {
                let _ = stream.shutdown(std::net::Shutdown::Write);
                return;
            };
            let bytes = match entry {
                Queued::Frame(bytes) => Some(bytes),
                Queued::Tick(tick) => tick_frames(*tick),
            };
            if bytes.is_none_or(|bytes| stream.write_all(&bytes).is_err()) {
                self.close();
                return;
            }
        }
    }
}

/// A tick's snapshot frame, carrying the ticks it replaced, then its events
/// frame; nothing when they would not encode.
fn tick_frames(queued: QueuedTick) -> Option<Vec<u8>> {
    let mut bytes = queued.snapshot;
    if !queued.skipped.is_empty() {
        let ServerMsg::Snapshot { mut view } =
            decode_payload::<ServerMsg>(bytes.get(LEN_PREFIX..)?).ok()?
        else {
            return None;
        };
        assert!(view.skipped_ticks.is_empty());
        assert_eq!(view.tick, queued.tick);
        view.skipped_ticks = queued.skipped;
        bytes = encode_frame_to_vec(&ServerMsg::Snapshot { view }).ok()?;
    }
    let events = ServerMsg::Events {
        tick: queued.tick,
        events: queued.events,
    };
    encode_frame(&events, &mut bytes).ok()?;
    Some(bytes)
}
