//! The outgoing side of one connection.

use std::collections::VecDeque;
use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex};

/// Frames waiting to leave on one connection.
///
/// The simulation thread pushes and never blocks; a writer thread drains onto
/// the socket at whatever pace the peer accepts. Snapshots are coalescible:
/// queueing a new one removes any older snapshot still waiting, so a slow
/// consumer skips states rather than falling behind. Everything else is kept
/// in order and never dropped; a queue that overflows closes the connection
/// instead.
pub struct Outbox {
    state: Mutex<OutboxState>,
    wake: Condvar,
}

struct OutboxState {
    queue: VecDeque<QueuedFrame>,
    closed: bool,
}

struct QueuedFrame {
    snapshot: bool,
    bytes: Vec<u8>,
}

/// Queued frames past this many close the connection as hopeless.
const QUEUE_LIMIT: usize = 1024;

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
        self.enqueue(QueuedFrame {
            snapshot: false,
            bytes,
        });
    }

    /// Queues a snapshot, replacing any snapshot still waiting.
    pub fn push_snapshot(&self, bytes: Vec<u8>) {
        self.enqueue(QueuedFrame {
            snapshot: true,
            bytes,
        });
    }

    fn enqueue(&self, frame: QueuedFrame) {
        let mut state = self.state.lock().expect("outbox lock");
        if state.closed {
            return;
        }
        if frame.snapshot {
            state.queue.retain(|f| !f.snapshot);
        }
        state.queue.push_back(frame);
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

    /// Whether the outbox has been closed.
    pub fn is_closed(&self) -> bool {
        self.state.lock().expect("outbox lock").closed
    }

    /// The writer thread body: drains frames onto the socket until the outbox
    /// closes and empties, or the socket fails.
    pub fn run_writer(self: &Arc<Outbox>, mut stream: TcpStream) {
        loop {
            let frame = {
                let mut state = self.state.lock().expect("outbox lock");
                loop {
                    if let Some(frame) = state.queue.pop_front() {
                        break Some(frame);
                    }
                    if state.closed {
                        break None;
                    }
                    state = self.wake.wait(state).expect("outbox lock");
                }
            };
            let Some(frame) = frame else {
                // The sending half only: shutting both down with our bytes
                // still in flight resets the connection and loses them.
                let _ = stream.shutdown(std::net::Shutdown::Write);
                return;
            };
            if stream.write_all(&frame.bytes).is_err() {
                self.close();
                return;
            }
        }
    }
}
