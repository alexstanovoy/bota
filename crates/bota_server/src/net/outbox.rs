//! The outgoing side of one connection.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bota_proto::{
    EventKind, LEN_PREFIX, MAX_SKIPPED_TICKS, ServerMsg, SkippedTick, decode_payload, encode_frame,
    encode_frame_to_vec,
};

use crate::net::FrameSink;
use crate::net::deflate::Deflater;

/// How often the writer sends [`ServerMsg::Ping`], whatever else it sends.
pub const PING_INTERVAL: Duration = Duration::from_secs(1);

/// How long a connection the server lets go is read on after its hang-up, for the peer to read
/// to the end and hang up too: a socket closed with bytes unread resets the connection, and the
/// peer would lose what it has not read yet. A peer still talking then is cut off, so a peer let
/// go cannot keep its connection (and its thread) by talking on.
const LINGER: Duration = Duration::from_secs(10);

/// How a connection the server closes parts, once what is queued has gone out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parting {
    /// A peer let go while the server runs on: read on for at most [`LINGER`].
    LetGo,
    /// The end of the match: read on until the peer hangs up, however long its line takes to
    /// deliver the end. The server takes no one after it, so nothing piles up.
    MatchOver,
}

/// Frames waiting to leave on one connection.
///
/// The simulation thread pushes and never blocks; a writer thread drains onto
/// the socket at whatever pace the peer accepts, and pings the peer every
/// [`PING_INTERVAL`] so it hears from the server even when nothing happens. A
/// tick, its snapshot and its events, is coalescible: queueing a new one
/// replaces any older tick still waiting, whose events then ride along in the
/// new snapshot's [`skipped_ticks`](bota_proto::WorldView::skipped_ticks), so
/// a slow consumer skips states but no events. The first tick queued is never
/// replaced: it is the state the peer starts from, and a peer still reading
/// the opening when the match moves on would otherwise start further in.
/// Everything else is kept in
/// order and never dropped; a queue that overflows, or a peer that skips more
/// than [`MAX_SKIPPED_TICKS`] in a row, aborts the connection instead.
///
/// Once the peer asks for compression, everything queued after the request
/// goes out compressed, pings among it, but chat: a line an enemy chose must
/// never share a compression window with a side's own state, or the sizes of
/// what follows it would tell whether it guessed some of that state.
pub struct Outbox {
    state: Mutex<OutboxState>,
    wake: Condvar,
    /// Wakes whoever waits for the writer to be done sending or for the reader to end.
    parting: Condvar,
}

struct OutboxState {
    queue: VecDeque<Queued>,
    /// Frames that go out before the queue: answers to probes.
    urgent: VecDeque<Vec<u8>>,
    /// Drain what is queued, then hang up and part so.
    closed: Option<Parting>,
    /// Drop what is queued and stop at once: the peer is gone or hopeless.
    aborted: bool,
    /// Whether a tick was ever queued: the first one is kept whole.
    ticked: bool,
    /// The writer sends no more: it hung up, or stopped.
    sent: bool,
    /// The connection's reader has ended.
    reader_ended: bool,
}

/// What the writer does next.
enum Step {
    Send(Queued),
    Ping,
    /// The queue is drained after a close: hang up, and part so.
    HangUp(Parting),
    Abort,
}

/// One entry of the queue.
enum Queued {
    /// A frame that must arrive as it is.
    Frame(Vec<u8>),
    /// A tick: its snapshot frame, its events and the ticks it replaced.
    Tick(Box<QueuedTick>),
    /// A chat frame, never compressed.
    Chat(Vec<u8>),
    /// Compress everything after this.
    Compress,
}

struct QueuedTick {
    /// The encoded snapshot of the tick, without skipped ticks.
    snapshot: Vec<u8>,
    tick: u32,
    events: Vec<EventKind>,
    /// Ticks this one replaced, oldest first.
    skipped: Vec<SkippedTick>,
    /// Whether it is the first tick the peer is sent, which no later one
    /// replaces.
    first: bool,
}

/// Urgent frames held for the writer at most; a probe past them goes unanswered, as a lost
/// packet would.
const URGENT_LIMIT: usize = 8;

/// Queued entries past this many close the connection as hopeless.
const QUEUE_LIMIT: usize = 1024;

impl Outbox {
    /// An empty, open outbox.
    pub fn new() -> Arc<Outbox> {
        Arc::new(Outbox {
            state: Mutex::new(OutboxState {
                queue: VecDeque::new(),
                urgent: VecDeque::new(),
                closed: None,
                aborted: false,
                ticked: false,
                sent: false,
                reader_ended: false,
            }),
            wake: Condvar::new(),
            parting: Condvar::new(),
        })
    }

    /// Queues a frame that must arrive.
    pub fn push(&self, bytes: Vec<u8>) {
        self.enqueue(Queued::Frame(bytes));
    }

    /// Queues a frame to go out before everything queued, behind only the frame the writer is
    /// sending now, and wakes the writer: for what a peer times, which must not wait its turn.
    /// Dropped when the peer already lags that many.
    pub fn push_urgent(&self, bytes: Vec<u8>) {
        let mut state = self.lock();
        if state.closed.is_some() || state.aborted || state.urgent.len() >= URGENT_LIMIT {
            return;
        }
        state.urgent.push_back(bytes);
        drop(state);
        self.wake.notify_one();
    }

    /// Queues a chat frame, which never goes out compressed.
    pub fn push_chat(&self, bytes: Vec<u8>) {
        self.enqueue(Queued::Chat(bytes));
    }

    /// Compresses everything queued after this (chat aside); once is enough.
    pub fn compress(&self) {
        self.enqueue(Queued::Compress);
    }

    /// Queues a tick, its encoded snapshot and its events, replacing any tick
    /// still waiting.
    pub fn push_tick(&self, snapshot: Vec<u8>, tick: u32, events: Vec<EventKind>) {
        self.enqueue(Queued::Tick(Box::new(QueuedTick {
            snapshot,
            tick,
            events,
            skipped: Vec::new(),
            first: false,
        })));
    }

    fn enqueue(&self, mut entry: Queued) {
        let mut state = self.lock();
        if state.closed.is_some() || state.aborted {
            return;
        }
        if let Queued::Tick(newer) = &mut entry {
            newer.first = !state.ticked;
            state.ticked = true;
        }
        if let Queued::Tick(newer) = &mut entry
            && let Some(at) = state
                .queue
                .iter()
                .position(|queued| matches!(queued, Queued::Tick(tick) if !tick.first))
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
                state.aborted = true;
            }
        }
        state.queue.push_back(entry);
        if state.queue.len() > QUEUE_LIMIT {
            state.aborted = true;
        }
        drop(state);
        self.wake.notify_one();
    }

    /// Lets the writer drain what is queued, then hang up and part as `parting` says.
    pub fn close(&self, parting: Parting) {
        self.lock().closed = Some(parting);
        self.wake.notify_one();
    }

    /// Drops what is queued and stops the writer without a word to the peer.
    pub fn abort(&self) {
        let mut state = self.lock();
        state.aborted = true;
        state.queue.clear();
        state.urgent.clear();
        drop(state);
        self.wake.notify_one();
    }

    /// Waits until the writer sends no more: after a [`close`](Outbox::close), what was queued
    /// has reached the socket.
    pub fn wait_sent(&self) {
        let mut state = self.lock();
        while !state.sent {
            state = self.parting.wait(state).expect("outbox lock");
        }
    }

    /// Tells a writer that reads on after its hang-up that the reader has ended.
    pub fn reader_ended(&self) {
        self.lock().reader_ended = true;
        self.parting.notify_all();
    }

    /// Whether nothing waits for the writer: it has taken all there was.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.lock().queue.is_empty()
    }

    fn lock(&self) -> MutexGuard<'_, OutboxState> {
        self.state.lock().expect("outbox lock")
    }

    /// Waits for the writer's next step: a ping when one is due, else the
    /// next queued entry.
    fn next_step(&self, ping_due: Instant) -> Step {
        let mut state = self.lock();
        loop {
            if state.aborted {
                return Step::Abort;
            }
            let now = Instant::now();
            if now >= ping_due {
                return Step::Ping;
            }
            if let Some(bytes) = state.urgent.pop_front() {
                return Step::Send(Queued::Frame(bytes));
            }
            if let Some(entry) = state.queue.pop_front() {
                return Step::Send(entry);
            }
            if let Some(parting) = state.closed {
                return Step::HangUp(parting);
            }
            state = self
                .wake
                .wait_timeout(state, ping_due - now)
                .expect("outbox lock")
                .0;
        }
    }

    /// Marks the writer done sending, and wakes whoever waits for it. A lock
    /// poisoned by a writer's panic still marks it.
    fn mark_sent(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sent = true;
        self.parting.notify_all();
    }

    /// Waits after a hang-up for the reader to end, at most [`LINGER`].
    fn linger(&self) {
        let until = Instant::now() + LINGER;
        let mut state = self.lock();
        while !state.reader_ended {
            let Some(left) = until.checked_duration_since(Instant::now()) else {
                return;
            };
            state = self
                .parting
                .wait_timeout(state, left)
                .expect("outbox lock")
                .0;
        }
    }

    /// The writer thread body: drains frames into the sink until the outbox
    /// closes and empties, then hangs up and lets the reader drain the
    /// connection, for at most [`LINGER`] when the peer is let go. Then, or at
    /// once when the outbox is aborted or the sink fails, it shuts the
    /// connection's reading side, so the connection's reader reports the peer
    /// gone.
    pub fn run_writer(self: &Arc<Outbox>, mut sink: Box<dyn FrameSink>) {
        let _sent = SentOnExit(self);
        let ping = encode_frame_to_vec(&ServerMsg::Ping).expect("a ping encodes");
        let mut ping_due = Instant::now() + PING_INTERVAL;
        let mut deflater = None;
        loop {
            let bytes = match self.next_step(ping_due) {
                Step::Send(Queued::Frame(bytes)) => Some(bytes),
                Step::Send(Queued::Tick(tick)) => tick_frames(*tick),
                Step::Send(Queued::Chat(bytes)) => {
                    send_or_abort(self, &mut *sink, Some(bytes));
                    continue;
                }
                Step::Send(Queued::Compress) => {
                    deflater.get_or_insert_with(Deflater::new);
                    continue;
                }
                Step::Ping => {
                    ping_due = Instant::now() + PING_INTERVAL;
                    Some(ping.clone())
                }
                Step::HangUp(parting) => {
                    sink.hang_up();
                    self.mark_sent();
                    if parting == Parting::MatchOver {
                        return;
                    }
                    self.linger();
                    break;
                }
                Step::Abort => break,
            };
            let bytes = match &mut deflater {
                Some(deflater) => bytes.and_then(|bytes| deflater.pack(&bytes)),
                None => bytes,
            };
            if !send_or_abort(self, &mut *sink, bytes) {
                break;
            }
        }
        sink.shut_read();
    }
}

/// Marks the writer done sending however it stops, a panic included, so
/// nobody waits for it for ever.
struct SentOnExit<'a>(&'a Outbox);

impl Drop for SentOnExit<'_> {
    fn drop(&mut self) {
        self.0.mark_sent();
    }
}

/// Sends frames, or aborts the outbox when there are none (they would not
/// encode) or the sink failed; whether they went out.
fn send_or_abort(outbox: &Outbox, sink: &mut dyn FrameSink, bytes: Option<Vec<u8>>) -> bool {
    if bytes.is_some_and(|bytes| sink.send_frames(bytes).is_ok()) {
        return true;
    }
    outbox.abort();
    false
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
