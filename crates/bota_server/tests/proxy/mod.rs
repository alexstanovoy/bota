//! A userspace TCP proxy that makes a line bad on purpose: latency and jitter,
//! a bandwidth cap, stalls, a black hole, and resets. It runs in the test's
//! own process and needs no privileges.
//!
//! Each connection accepted is piped to the server by two pumps, one per
//! direction. A pump reads a chunk, holds it until its delivery time
//! (latency plus jitter, never before the chunk ahead of it, as on one TCP
//! stream) and writes it on at no more than the cap. A stalled pump neither
//! reads nor writes, so the sender meets the backpressure a stalled line
//! makes. A reset closes both sockets with a zero linger: each end sees its
//! connection reset, not closed.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How often a waiting pump looks at its controls.
const POLL: Duration = Duration::from_millis(5);
/// Bytes a pump reads at once.
const CHUNK: usize = 8 * 1024;
/// The receive buffer of each proxied socket, in bytes.
const RECV_BUFFER: usize = 64 * 1024;
/// Chunks held in flight per direction; past them the pump stops reading.
const IN_FLIGHT: usize = 256;

/// Which way bytes flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// From the client to the server.
    Up,
    /// From the server to the client.
    Down,
}

/// What the line does to the bytes of one direction.
#[derive(Clone, Copy, Debug, Default)]
pub struct Line {
    /// Added to every chunk on its way.
    pub latency: Duration,
    /// Up to this much more, drawn per chunk.
    pub jitter: Duration,
    /// The most bytes a second; none for no cap.
    pub bytes_per_sec: Option<u64>,
}

/// The controls of one piped connection.
#[derive(Default)]
struct Pipe {
    stalled: [AtomicBool; 2],
    reset: AtomicBool,
}

/// A proxy in front of one server address.
pub struct Proxy {
    addr: SocketAddr,
    lines: Arc<Mutex<[Line; 2]>>,
    pipes: Arc<Mutex<Vec<Arc<Pipe>>>>,
    accepted: Arc<AtomicUsize>,
}

impl Proxy {
    /// Listens on a free local port and pipes every connection to `server`.
    pub fn start(server: SocketAddr) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port for the proxy");
        let proxy = Proxy {
            addr: listener.local_addr().expect("bound"),
            lines: Arc::default(),
            pipes: Arc::default(),
            accepted: Arc::default(),
        };
        let (lines, pipes, accepted) = (
            Arc::clone(&proxy.lines),
            Arc::clone(&proxy.pipes),
            Arc::clone(&proxy.accepted),
        );
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let Ok(upstream) = TcpStream::connect(server) else {
                    continue;
                };
                accepted.fetch_add(1, Ordering::SeqCst);
                let pipe = Arc::new(Pipe::default());
                pipes.lock().expect("pipes").push(Arc::clone(&pipe));
                connect(client, upstream, &pipe, &lines);
            }
        });
        proxy
    }

    /// The address clients dial.
    pub fn addr(&self) -> String {
        self.addr.to_string()
    }

    /// Connections taken so far.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Sets what the line does to one direction, from now on.
    pub fn set_line(&self, dir: Dir, line: Line) {
        self.lines.lock().expect("lines")[dir as usize] = line;
    }

    /// Stalls or frees one direction of every connection piped so far;
    /// connections made later flow.
    pub fn stall(&self, dir: Dir, stalled: bool) {
        for pipe in self.pipes.lock().expect("pipes").iter() {
            pipe.stalled[dir as usize].store(stalled, Ordering::SeqCst);
        }
    }

    /// Resets every connection piped so far.
    pub fn reset(&self) {
        for pipe in self.pipes.lock().expect("pipes").drain(..) {
            pipe.reset.store(true, Ordering::SeqCst);
        }
    }
}

/// Starts the two pumps of one connection.
fn connect(
    client: TcpStream,
    upstream: TcpStream,
    pipe: &Arc<Pipe>,
    lines: &Arc<Mutex<[Line; 2]>>,
) {
    for stream in [&client, &upstream] {
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(POLL));
        // A fixed, small receive buffer: a stalled pump's backpressure reaches the sender within
        // a few snapshots, however large the kernel would let the buffer grow.
        let _ = socket2::SockRef::from(stream).set_recv_buffer_size(RECV_BUFFER);
    }
    let ends = [
        (Dir::Up, client.try_clone(), upstream.try_clone()),
        (Dir::Down, upstream.try_clone(), client.try_clone()),
    ];
    for (dir, from, to) in ends {
        let (Ok(from), Ok(to)) = (from, to) else {
            pipe.reset.store(true, Ordering::SeqCst);
            return;
        };
        pump(dir, from, to, Arc::clone(pipe), Arc::clone(lines));
    }
}

/// A chunk on its way, or the sender's clean close.
enum Carried {
    Bytes(Vec<u8>, Instant),
    Closed,
}

/// One direction: a reader thread holding chunks back by the line, and a
/// writer thread delivering them.
fn pump(dir: Dir, from: TcpStream, to: TcpStream, pipe: Arc<Pipe>, lines: Arc<Mutex<[Line; 2]>>) {
    let (tx, rx) = sync_channel(IN_FLIGHT);
    let reader_pipe = Arc::clone(&pipe);
    thread::spawn(move || read_side(dir, from, &reader_pipe, &lines, &tx));
    thread::spawn(move || {
        let mut to = to;
        loop {
            if pipe.reset.load(Ordering::SeqCst) {
                return abandon(&to);
            }
            let carried = match rx.recv_timeout(POLL) {
                Ok(carried) => carried,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return abandon(&to),
            };
            let (bytes, due) = match carried {
                Carried::Bytes(bytes, due) => (bytes, due),
                Carried::Closed => {
                    let _ = to.shutdown(Shutdown::Write);
                    return;
                }
            };
            if !wait_until(&pipe, dir, due) || to.write_all(&bytes).is_err() {
                pipe.reset.store(true, Ordering::SeqCst);
                return abandon(&to);
            }
        }
    });
}

/// Reads chunks while the direction flows, stamping each with its delivery
/// time: the line's latency and jitter, then the cap spread over the chunks
/// in order. A clean close is passed on, a failure resets the connection.
fn read_side(
    dir: Dir,
    mut from: TcpStream,
    pipe: &Pipe,
    lines: &Mutex<[Line; 2]>,
    tx: &SyncSender<Carried>,
) {
    let mut buf = vec![0u8; CHUNK];
    let mut last_due = Instant::now();
    let mut seed = 0x9e37_79b9_7f4a_7c15 ^ dir as u64;
    loop {
        if pipe.reset.load(Ordering::SeqCst) {
            return abandon(&from);
        }
        if pipe.stalled[dir as usize].load(Ordering::SeqCst) {
            thread::sleep(POLL);
            continue;
        }
        let n = match from.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(Carried::Closed);
                return;
            }
            Ok(n) => n,
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                continue;
            }
            Err(_) => {
                pipe.reset.store(true, Ordering::SeqCst);
                return abandon(&from);
            }
        };
        let line = lines.lock().expect("lines")[dir as usize];
        let jitter = line.jitter * (next_random(&mut seed) % 1024) / 1024;
        last_due = last_due.max(Instant::now() + line.latency + jitter);
        last_due += line.bytes_per_sec.map_or(Duration::ZERO, |cap| {
            Duration::from_nanos(n as u64 * 1_000_000_000 / cap.max(1))
        });
        if tx
            .send(Carried::Bytes(buf[..n].to_vec(), last_due))
            .is_err()
        {
            return abandon(&from);
        }
    }
}

/// Waits until `due` while the direction is not stalled; false once the
/// connection is reset.
fn wait_until(pipe: &Pipe, dir: Dir, due: Instant) -> bool {
    loop {
        if pipe.reset.load(Ordering::SeqCst) {
            return false;
        }
        let now = Instant::now();
        if now >= due && !pipe.stalled[dir as usize].load(Ordering::SeqCst) {
            return true;
        }
        thread::sleep(
            POLL.min(due.saturating_duration_since(now))
                .max(Duration::from_micros(200)),
        );
    }
}

/// Lets go of one end so that, once every handle is gone, the peer is reset.
fn abandon(stream: &TcpStream) {
    let _ = socket2::SockRef::from(stream).set_linger(Some(Duration::ZERO));
}

/// The next number of a xorshift stream.
fn next_random(state: &mut u64) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 32) as u32
}
