//! The two socket [`Transport`]s: the blocking [`Link`] with timeouts, and a
//! threaded [`Net`] whose poll never blocks the caller. Both answer the
//! server's pings themselves and dial the server again after the connection
//! breaks, by a [`Redial`] policy.
#![allow(clippy::disallowed_types)] // the connect deadline and receive timing read the clock

mod frames;
mod tcp;
#[cfg(feature = "websocket")]
mod ws;

use std::io::ErrorKind;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{ClientMsg, ServerMsg};

use crate::ping::PingMeter;
use crate::receipts::{OrderReceipts, Receipt};
use crate::redial::{Redial, Rejoin, redial, seat_held};
use crate::transport::{DEFAULT_IO_TIMEOUT, Transport};
pub(crate) use frames::FrameSink;
use frames::{FrameSource, FrameStream, send_message};

pub use frames::Endpoint;
pub use tcp::open_tcp;
#[cfg(feature = "websocket")]
pub use ws::TlsConfig;

/// Messages the threaded reader holds for its caller at most. Past them it
/// stops reading, and the server skips ticks for it as for any slow reader.
const MAX_INBOX: usize = 1024;
/// Payload bytes of the messages the threaded reader holds at most, past
/// which it stops reading as well. A decoded message takes up to some
/// sixteen times its payload, and a payload may be 4 MiB: counted in
/// messages alone, what waits for a caller that stalled could be gigabytes.
const MAX_INBOX_BYTES: usize = 16 * 1024 * 1024;
/// How often a reader waiting for room looks whether its caller is gone.
const INBOX_RECHECK: Duration = Duration::from_millis(100);

/// What the reader hands its caller, with the payload bytes it holds.
type Inbound = (std::io::Result<ServerMsg>, usize);

/// One framed connection that blocks on every read, up to a timeout.
///
/// A connection that breaks (an error, the timeout, a reset) is dialed again
/// inside [`Transport::recv`], and the stream goes on with the server's
/// welcome to the fresh connection; a send that fails is kept for it. A clean
/// close by the server is the end.
pub struct Link {
    endpoint: Endpoint,
    timeout: Duration,
    redial: Redial,
    rejoin: Rejoin,
    stream: Box<dyn FrameStream>,
    receive_wait: Duration,
    /// Why the connection broke while sending; the next receive dials again.
    broken: Option<std::io::Error>,
}

impl Link {
    /// Connects with one timeout for establishing and using the connection;
    /// the server pings every second, so a link silent for that long is
    /// broken. A break is dialed again by [`Redial::DEFAULT`]. `address` is
    /// an [`Endpoint`]'s text: TCP `host:port`, or a `ws://` / `wss://` URL.
    pub fn connect(address: &str, timeout: Duration) -> std::io::Result<Self> {
        Self::open(Endpoint::parse(address)?, timeout)
    }

    /// Connects to `endpoint`, as [`Link::connect`] does.
    pub fn open(endpoint: Endpoint, timeout: Duration) -> std::io::Result<Self> {
        if timeout.is_zero() {
            return Err(std::io::Error::other("server I/O timeout must be positive"));
        }
        Ok(Self {
            stream: endpoint.open(timeout, timeout)?,
            endpoint,
            timeout,
            redial: Redial::DEFAULT,
            rejoin: Rejoin::new(),
            receive_wait: Duration::ZERO,
            broken: None,
        })
    }

    /// The same link, dialing again by `redial` after a break;
    /// [`Redial::OFF`] makes a break the caller's error.
    pub fn with_redial(mut self, redial: Redial) -> Self {
        self.redial = redial;
        self
    }

    fn read_message(&mut self) -> std::io::Result<Option<ServerMsg>> {
        let read = self.stream.read_message();
        self.receive_wait = self.receive_wait.saturating_add(self.stream.take_wait());
        read
    }

    /// Dials again after `cause`, when that is wanted.
    fn reconnect(&mut self, cause: std::io::Error) -> std::io::Result<()> {
        if !may_redial(self.redial, &self.rejoin, &cause) {
            return Err(cause);
        }
        let (endpoint, timeout) = (&self.endpoint, self.timeout);
        let mut spent = self.rejoin.spent;
        let rejoin = &mut self.rejoin;
        let mut fresh = None;
        let dialed = redial(
            self.redial,
            &mut spent,
            cause,
            || endpoint.open(timeout, timeout),
            |mut stream| {
                rejoin.greet(&mut *stream)?;
                fresh = Some(stream);
                Ok(())
            },
        );
        self.rejoin.spent = spent;
        dialed?;
        self.stream = fresh.expect("redial adopted a stream");
        Ok(())
    }
}

impl Transport for Link {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.receive_wait = Duration::ZERO;
        loop {
            let cause = match self.broken.take() {
                Some(cause) => cause,
                None => match self.read_message() {
                    Ok(Some(ServerMsg::Ping)) => {
                        self.broken = send_message(&mut *self.stream, &ClientMsg::Pong).err();
                        continue;
                    }
                    Ok(Some(message)) if self.rejoin.heard(&message) => seat_held(),
                    Ok(Some(message)) => return Ok(Some(message)),
                    Ok(None) => return Ok(None),
                    Err(cause) => cause,
                },
            };
            self.reconnect(cause)?;
        }
    }

    fn take_receive_wait(&mut self) -> Option<Duration> {
        Some(std::mem::take(&mut self.receive_wait))
    }

    fn send(&mut self, message: &ClientMsg) -> std::io::Result<()> {
        let message = outgoing(self.redial, &mut self.rejoin, message);
        match send_message(&mut *self.stream, &message) {
            Err(error) if may_redial(self.redial, &self.rejoin, &error) => {
                self.rejoin.unsent(message);
                self.broken = Some(error);
                Ok(())
            }
            sent => sent,
        }
    }
}

/// The message as it goes out: noted for a later connection when the
/// transport dials again.
fn outgoing(policy: Redial, rejoin: &mut Rejoin, message: &ClientMsg) -> ClientMsg {
    if policy.attempts > 0 {
        rejoin.outgoing(message)
    } else {
        message.clone()
    }
}

/// Whether a connection broken by `cause` is dialed again: the policy allows
/// it, the match has not ended, and the server's stream was not corrupt.
fn may_redial(policy: Redial, rejoin: &Rejoin, cause: &std::io::Error) -> bool {
    policy.attempts > 0 && rejoin.wanted() && cause.kind() != ErrorKind::InvalidData
}

/// A socket with a reader thread behind it.
///
/// Messages wait in a bounded channel; [`Net::poll`] hands over whatever has
/// arrived without ever blocking the render loop, and [`Transport::recv`]
/// waits for the next one. The reader answers the server's pings, and after
/// a break dials again in the background while the caller goes on: sends
/// meanwhile are kept for the fresh connection, and [`Net::is_redialing`]
/// says so.
pub struct Net {
    shared: Arc<Shared>,
    /// What the reader read, then the error that stopped it, if one did.
    inbox: Receiver<Inbound>,
    /// The error [`Net::poll`] met, kept for the next [`Transport::recv`].
    failure: Option<std::io::Error>,
}

const UP: u8 = 0;
const REDIALING: u8 = 1;
const CLOSED: u8 = 2;

/// The writing half of a [`Net`] and its state, shared by the caller and the
/// reader thread.
struct Shared {
    out: Mutex<Outgoing>,
    state: AtomicU8,
    /// Payload bytes of the messages in the inbox.
    held: Mutex<usize>,
    /// Signalled when the caller takes messages out of the inbox.
    taken: Condvar,
    /// The clock the meter is stamped on: monotonic, from the connection's start.
    epoch: Instant,
    /// The round trip, timed with probes and their echoes.
    meter: Mutex<PingMeter>,
    /// The orders sent and their receipts.
    receipts: Mutex<OrderReceipts>,
}

struct Outgoing {
    sink: Box<dyn FrameSink>,
    redial: Redial,
    rejoin: Rejoin,
    /// A send failed: the reader dials again rather than take the end of
    /// its stream for a clean close.
    broken: bool,
}

impl Outgoing {
    /// Sends one message; one that fails while a redial will follow is kept
    /// for the fresh connection and is no error.
    fn send(&mut self, message: &ClientMsg) -> std::io::Result<()> {
        let message = outgoing(self.redial, &mut self.rejoin, message);
        match send_message(&mut *self.sink, &message) {
            Err(error) if may_redial(self.redial, &self.rejoin, &error) => {
                self.rejoin.unsent(message);
                self.break_off();
                Ok(())
            }
            sent => sent,
        }
    }

    /// Marks the connection broken and wakes the reader.
    fn break_off(&mut self) {
        self.broken = true;
        self.sink.abort();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Outgoing> {
        self.out.lock().expect("net lock")
    }

    /// The server answers on the fresh connection: the redial is over. A
    /// connection that opened but is not answered (a black hole, a held
    /// seat) is still being redialed.
    /// Queues a message for the caller once the inbox has room for its bytes
    /// (an empty inbox takes any one); false when the caller is gone.
    fn hand_over(self: &Arc<Self>, tx: &SyncSender<Inbound>, msg: ServerMsg) -> bool {
        let bytes = bota_proto::payload_len(&msg).unwrap_or(bota_proto::MAX_PAYLOAD_LEN);
        let mut held = self.held.lock().expect("inbox lock");
        while *held > 0 && *held + bytes > MAX_INBOX_BYTES {
            if Arc::strong_count(self) == 1 {
                return false;
            }
            held = self
                .taken
                .wait_timeout(held, INBOX_RECHECK)
                .expect("inbox lock")
                .0;
        }
        *held += bytes;
        drop(held);
        tx.send((Ok(msg), bytes)).is_ok()
    }

    /// The caller took messages of `bytes` payload bytes out of the inbox.
    fn release(&self, bytes: usize) {
        if bytes > 0 {
            *self.held.lock().expect("inbox lock") -= bytes;
            self.taken.notify_one();
        }
    }

    fn heard_from_server(&self) {
        let _ = self
            .state
            .compare_exchange(REDIALING, UP, Ordering::Relaxed, Ordering::Relaxed);
    }

    /// Sends a probe when one is due, stamped as the socket takes it.
    fn probe_if_due(&self) {
        let due = self
            .meter
            .lock()
            .expect("meter lock")
            .probe(self.epoch.elapsed());
        if let Some(probe) = due {
            let mut out = self.lock();
            if send_message(&mut *out.sink, &probe).is_err() {
                out.break_off();
            }
        }
    }

    /// Stamps the receipt of order `seq` the moment the reader has it.
    fn order_received(&self, seq: u32, tick: u32) {
        let now = self.epoch.elapsed();
        self.receipts
            .lock()
            .expect("receipts lock")
            .received(seq, tick, now);
    }

    /// Stamps the echo of probe `id` the moment the reader has it.
    fn echoed(&self, id: u32) {
        let now = self.epoch.elapsed();
        self.meter.lock().expect("meter lock").echoed(id, now);
    }

    fn answer_ping(&self) {
        let mut out = self.lock();
        if send_message(&mut *out.sink, &ClientMsg::Pong).is_err() {
            out.break_off();
        }
    }

    /// Dials again after `cause` when that is wanted, and returns the reading
    /// half of the fresh connection. The caller is never locked out for
    /// longer than a greeting takes.
    fn reconnect(
        &self,
        endpoint: &Endpoint,
        cause: std::io::Error,
    ) -> std::io::Result<Box<dyn FrameSource>> {
        let (policy, mut spent) = {
            let out = self.lock();
            if !may_redial(out.redial, &out.rejoin, &cause) {
                return Err(cause);
            }
            (out.redial, out.rejoin.spent)
        };
        self.state.store(REDIALING, Ordering::Relaxed);
        let mut fresh = None;
        let dialed = redial(
            policy,
            &mut spent,
            cause,
            || endpoint.open(DEFAULT_IO_TIMEOUT, DEFAULT_IO_TIMEOUT),
            |mut stream| {
                let mut out = self.lock();
                out.rejoin.greet(&mut *stream)?;
                let (reader, sink) = stream.split()?;
                out.sink = sink;
                out.broken = false;
                fresh = Some(reader);
                Ok(())
            },
        );
        self.lock().rejoin.spent = spent;
        dialed?;
        Ok(fresh.expect("redial adopted a stream"))
    }
}

impl Net {
    /// Connects and starts reading; a break is dialed again by
    /// [`Redial::DEFAULT`].
    pub fn connect(addr: &str) -> std::io::Result<Net> {
        Self::connect_with(addr, Redial::DEFAULT)
    }

    /// Connects and starts reading; a break is dialed again by `redial`.
    pub fn connect_with(addr: &str, redial: Redial) -> std::io::Result<Net> {
        let endpoint = Endpoint::parse(addr)?;
        Self::open(endpoint, redial)
    }

    /// Connects to `endpoint` and starts reading; a break is dialed again by
    /// `redial`.
    pub fn open(endpoint: Endpoint, redial: Redial) -> std::io::Result<Net> {
        let stream = endpoint.open(DEFAULT_IO_TIMEOUT, DEFAULT_IO_TIMEOUT)?;
        let (reader, sink) = stream.split()?;
        let (tx, inbox) = sync_channel(MAX_INBOX);
        let shared = Arc::new(Shared {
            out: Mutex::new(Outgoing {
                sink,
                redial,
                rejoin: Rejoin::new(),
                broken: false,
            }),
            state: AtomicU8::new(UP),
            held: Mutex::new(0),
            taken: Condvar::new(),
            epoch: Instant::now(),
            meter: Mutex::new(PingMeter::default()),
            receipts: Mutex::new(OrderReceipts::default()),
        });
        let thread_shared = Arc::clone(&shared);
        thread::spawn(move || read_on(thread_shared, &endpoint, reader, &tx));
        Ok(Net {
            shared,
            inbox,
            failure: None,
        })
    }

    /// Everything that arrived since the last poll.
    pub fn poll(&mut self) -> Vec<ServerMsg> {
        self.shared.probe_if_due();
        let mut messages = Vec::new();
        for (next, bytes) in self.inbox.try_iter() {
            self.shared.release(bytes);
            match next {
                Ok(msg) => messages.push(msg),
                Err(error) => self.failure = Some(error),
            }
        }
        messages
    }

    /// The smoothed round trip of the connection, once a probe has come back: timed by the
    /// socket (see [`PingMeter`]), so it moves in milliseconds, not in ticks or frames.
    pub fn ping(&self) -> Option<Duration> {
        self.shared.meter.lock().expect("meter lock").ping()
    }

    /// Orders sent that the server has not yet said it received.
    pub fn pending_orders(&self) -> usize {
        self.shared
            .receipts
            .lock()
            .expect("receipts lock")
            .pending()
    }

    /// The smoothed time from sending an order to hearing it was received, once one was.
    pub fn order_latency(&self) -> Option<Duration> {
        self.shared
            .receipts
            .lock()
            .expect("receipts lock")
            .smoothed()
    }

    /// The latest order receipt.
    pub fn last_receipt(&self) -> Option<Receipt> {
        self.shared.receipts.lock().expect("receipts lock").last()
    }

    /// Whether the connection has ended for good.
    pub fn is_closed(&self) -> bool {
        self.shared.state.load(Ordering::Relaxed) == CLOSED
    }

    /// Whether the connection broke and is not yet back: being dialed again,
    /// or dialed and not yet answered by the server.
    pub fn is_redialing(&self) -> bool {
        self.shared.state.load(Ordering::Relaxed) == REDIALING
    }
}

/// The reader thread: hands messages to the caller and answers pings, dials
/// again after a break, and ends after a clean close, a failure it does not
/// dial again after, or once the caller has let go of the [`Net`].
fn read_on(
    shared: Arc<Shared>,
    endpoint: &Endpoint,
    mut stream: Box<dyn FrameSource>,
    tx: &SyncSender<Inbound>,
) {
    let caller_gone = |shared: &Arc<Shared>| Arc::strong_count(shared) == 1;
    loop {
        let (mut gone, mut held) = (false, false);
        let read = read_messages(&mut *stream, |msg| {
            if matches!(msg, ServerMsg::Ping) {
                shared.heard_from_server();
                gone = caller_gone(&shared);
                shared.answer_ping();
                return !gone;
            }
            if let ServerMsg::OrderReceived { seq, tick, .. } = msg {
                shared.order_received(seq, tick);
                shared.heard_from_server();
                gone = caller_gone(&shared);
                return !gone;
            }
            if let ServerMsg::Echo { id } = msg {
                shared.echoed(id);
                shared.heard_from_server();
                gone = caller_gone(&shared);
                return !gone;
            }
            held = shared.lock().rejoin.heard(&msg);
            if !held {
                shared.heard_from_server();
            }
            gone = !held && !shared.hand_over(tx, msg);
            !gone && !held
        });
        let broken = std::mem::take(&mut shared.lock().broken);
        let cause = match read {
            _ if gone || caller_gone(&shared) => break,
            _ if held => seat_held(),
            Err(cause) => cause,
            Ok(()) if broken => {
                std::io::Error::new(ErrorKind::BrokenPipe, "failed to send client message")
            }
            Ok(()) => break,
        };
        match shared.reconnect(endpoint, cause) {
            Ok(fresh) => stream = fresh,
            Err(error) => {
                let _ = tx.send((Err(error), 0));
                break;
            }
        }
    }
    shared.state.store(CLOSED, Ordering::Relaxed);
}

/// Hands every message read to `take` until the server closes cleanly or
/// `take` declines one; a read or decode failure is the error.
fn read_messages(
    stream: &mut dyn FrameSource,
    mut take: impl FnMut(ServerMsg) -> bool,
) -> std::io::Result<()> {
    while let Some(msg) = stream.read_message()? {
        if !take(msg) {
            break;
        }
    }
    Ok(())
}

impl Transport for Net {
    /// Waits for the next message; none once the server closed cleanly, and
    /// the error once reading failed for good.
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        self.shared.probe_if_due();
        match self.inbox.recv() {
            Ok((next, bytes)) => {
                self.shared.release(bytes);
                next.map(Some)
            }
            Err(_) => Ok(None),
        }
    }

    /// Sends one message. A failed send the reader will not dial again after
    /// marks the connection closed.
    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        if let ClientMsg::Order { seq, .. } = msg {
            let now = self.shared.epoch.elapsed();
            self.shared
                .receipts
                .lock()
                .expect("receipts lock")
                .sent(*seq, now);
        }
        self.shared.lock().send(msg).inspect_err(|_| {
            self.shared.state.store(CLOSED, Ordering::Relaxed);
        })
    }
}

#[cfg(test)]
mod tests;
