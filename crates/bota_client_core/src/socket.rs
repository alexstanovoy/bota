//! The two socket [`Transport`]s: the blocking [`Link`] with timeouts, and a
//! threaded [`Net`] whose poll never blocks the caller. Both answer the
//! server's pings themselves and dial the server again after the connection
//! breaks, by a [`Redial`] policy.
#![allow(clippy::disallowed_types)] // the connect deadline and receive timing read the clock

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{ClientMsg, CodecError, FrameReader, ServerMsg, encode_frame_to_vec};

use crate::redial::{Redial, Rejoin, redial, seat_held};
use crate::transport::{DEFAULT_IO_TIMEOUT, Transport};

const READ_BUFFER_LEN: usize = 64 * 1024;
const MAX_RESOLVED_ADDRESSES: usize = 16;
/// Messages the threaded reader holds for its caller at most. Past them it
/// stops reading, and the server skips ticks for it as for any slow reader.
const MAX_INBOX: usize = 1024;

/// One framed TCP connection that blocks on every read, up to a timeout.
///
/// A connection that breaks (an error, the timeout, a reset) is dialed again
/// inside [`Transport::recv`], and the stream goes on with the server's
/// welcome to the fresh connection; a send that fails is kept for it. A clean
/// close by the server is the end.
pub struct Link {
    address: String,
    timeout: Duration,
    redial: Redial,
    rejoin: Rejoin,
    stream: TcpStream,
    reader: FrameReader,
    buffer: Box<[u8]>,
    receive_wait: Duration,
    /// Why the connection broke while sending; the next receive dials again.
    broken: Option<std::io::Error>,
}

impl Link {
    /// Connects with one timeout for establishing and using the connection;
    /// the server pings every second, so a link silent for that long is
    /// broken. A break is dialed again by [`Redial::DEFAULT`].
    pub fn connect(address: &str, timeout: Duration) -> std::io::Result<Self> {
        if timeout.is_zero() {
            return Err(std::io::Error::other("server I/O timeout must be positive"));
        }
        Ok(Self {
            stream: open(address, timeout)?,
            address: address.to_owned(),
            timeout,
            redial: Redial::DEFAULT,
            rejoin: Rejoin::new(),
            reader: FrameReader::new(),
            buffer: vec![0; READ_BUFFER_LEN].into_boxed_slice(),
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
        loop {
            if let Some(message) = self.reader.next_message().map_err(undecodable)? {
                return Ok(Some(message));
            }
            let started = Instant::now();
            let read = self.stream.read(&mut self.buffer);
            self.receive_wait = self.receive_wait.saturating_add(started.elapsed());
            let read = read.map_err(|error| io_context("failed to read server message", error))?;
            if read == 0 {
                return Ok(None);
            }
            self.reader.push(&self.buffer[..read]);
        }
    }

    /// Dials again after `cause`, when that is wanted.
    fn reconnect(&mut self, cause: std::io::Error) -> std::io::Result<()> {
        if !may_redial(self.redial, &self.rejoin, &cause) {
            return Err(cause);
        }
        let (address, timeout) = (&self.address, self.timeout);
        let mut spent = self.rejoin.spent;
        let rejoin = &mut self.rejoin;
        let mut fresh = None;
        let dialed = redial(
            self.redial,
            &mut spent,
            cause,
            || open(address, timeout),
            |mut stream| {
                rejoin.greet(&mut stream)?;
                fresh = Some(stream);
                Ok(())
            },
        );
        self.rejoin.spent = spent;
        dialed?;
        self.stream = fresh.expect("redial adopted a stream");
        self.reader = FrameReader::new();
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
                        self.broken = write(&mut self.stream, &ClientMsg::Pong).err();
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
        match write(&mut self.stream, &message) {
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

fn write(stream: &mut TcpStream, message: &ClientMsg) -> std::io::Result<()> {
    let frame = encode_frame_to_vec(message).map_err(|error| {
        std::io::Error::other(format!("failed to encode client message: {error}"))
    })?;
    stream
        .write_all(&frame)
        .map_err(|error| io_context("failed to send client message", error))
}

/// Connects with one timeout for establishing and using the connection.
fn open(address: &str, timeout: Duration) -> std::io::Result<TcpStream> {
    let stream = connect(address, timeout)?;
    stream
        .set_nodelay(true)
        .map_err(|error| io_context("failed to enable TCP_NODELAY", error))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| io_context("failed to set read timeout", error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| io_context("failed to set write timeout", error))?;
    Ok(stream)
}

fn connect(address: &str, timeout: Duration) -> std::io::Result<TcpStream> {
    let addresses = address.to_socket_addrs().map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("failed to resolve server address {address}: {error}"),
        )
    })?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| std::io::Error::other("server I/O timeout is too large"))?;
    let mut last_error = None;
    for resolved in addresses.take(MAX_RESOLVED_ADDRESSES) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&resolved, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    let detail = last_error.map_or_else(
        || "the address resolved to no usable socket".to_string(),
        |error| error.to_string(),
    );
    Err(std::io::Error::other(format!(
        "failed to connect to server at {address} within {timeout:?}: {detail}"
    )))
}

fn undecodable(error: CodecError) -> std::io::Error {
    std::io::Error::new(
        ErrorKind::InvalidData,
        format!(
            "failed to decode server message: {error}; likely a protocol mismatch, build client \
and server from matching commits"
        ),
    )
}

fn io_context(context: &str, error: std::io::Error) -> std::io::Error {
    std::io::Error::new(error.kind(), format!("{context}: {error}"))
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
    inbox: Receiver<std::io::Result<ServerMsg>>,
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
}

struct Outgoing {
    stream: TcpStream,
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
        match write(&mut self.stream, &message) {
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
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Outgoing> {
        self.out.lock().expect("net lock")
    }

    fn answer_ping(&self) {
        let mut out = self.lock();
        if write(&mut out.stream, &ClientMsg::Pong).is_err() {
            out.break_off();
        }
    }

    /// Dials again after `cause` when that is wanted, and returns the reading
    /// half of the fresh connection. The caller is never locked out for
    /// longer than a greeting takes.
    fn reconnect(&self, address: &str, cause: std::io::Error) -> std::io::Result<TcpStream> {
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
            || open(address, DEFAULT_IO_TIMEOUT),
            |mut stream| {
                let reader = stream.try_clone()?;
                let mut out = self.lock();
                out.rejoin.greet(&mut stream)?;
                out.stream = stream;
                out.broken = false;
                fresh = Some(reader);
                Ok(())
            },
        );
        self.lock().rejoin.spent = spent;
        dialed?;
        self.state.store(UP, Ordering::Relaxed);
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
        let stream = open(addr, DEFAULT_IO_TIMEOUT)?;
        let reader = stream.try_clone()?;
        let (tx, inbox) = sync_channel(MAX_INBOX);
        let shared = Arc::new(Shared {
            out: Mutex::new(Outgoing {
                stream,
                redial,
                rejoin: Rejoin::new(),
                broken: false,
            }),
            state: AtomicU8::new(UP),
        });
        let thread_shared = Arc::clone(&shared);
        let address = addr.to_owned();
        thread::spawn(move || read_on(thread_shared, &address, reader, &tx));
        Ok(Net {
            shared,
            inbox,
            failure: None,
        })
    }

    /// Everything that arrived since the last poll.
    pub fn poll(&mut self) -> Vec<ServerMsg> {
        let mut messages = Vec::new();
        for next in self.inbox.try_iter() {
            match next {
                Ok(msg) => messages.push(msg),
                Err(error) => self.failure = Some(error),
            }
        }
        messages
    }

    /// Whether the connection has ended for good.
    pub fn is_closed(&self) -> bool {
        self.shared.state.load(Ordering::Relaxed) == CLOSED
    }

    /// Whether the connection broke and is being dialed again.
    pub fn is_redialing(&self) -> bool {
        self.shared.state.load(Ordering::Relaxed) == REDIALING
    }
}

/// The reader thread: hands messages to the caller and answers pings, dials
/// again after a break, and ends after a clean close, a failure it does not
/// dial again after, or once the caller has let go of the [`Net`].
fn read_on(
    shared: Arc<Shared>,
    address: &str,
    mut stream: TcpStream,
    tx: &SyncSender<std::io::Result<ServerMsg>>,
) {
    let caller_gone = |shared: &Arc<Shared>| Arc::strong_count(shared) == 1;
    loop {
        let (mut gone, mut held) = (false, false);
        let read = read_messages(&mut stream, |msg| {
            if matches!(msg, ServerMsg::Ping) {
                gone = caller_gone(&shared);
                shared.answer_ping();
                return !gone;
            }
            held = shared.lock().rejoin.heard(&msg);
            gone = !held && tx.send(Ok(msg)).is_err();
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
        match shared.reconnect(address, cause) {
            Ok(fresh) => stream = fresh,
            Err(error) => {
                let _ = tx.send(Err(error));
                break;
            }
        }
    }
    shared.state.store(CLOSED, Ordering::Relaxed);
}

/// Hands every message read to `take` until the server closes cleanly or
/// `take` declines one; a read or decode failure is the error.
fn read_messages(
    stream: &mut TcpStream,
    mut take: impl FnMut(ServerMsg) -> bool,
) -> std::io::Result<()> {
    let mut reader = FrameReader::new();
    let mut buf = [0u8; READ_BUFFER_LEN];
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(io_context("failed to read server message", error)),
        };
        reader.push(&buf[..n]);
        while let Some(msg) = reader.next_message::<ServerMsg>().map_err(undecodable)? {
            if !take(msg) {
                return Ok(());
            }
        }
    }
}

impl Transport for Net {
    /// Waits for the next message; none once the server closed cleanly, and
    /// the error once reading failed for good.
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        match self.inbox.recv() {
            Ok(next) => next.map(Some),
            Err(_) => Ok(None),
        }
    }

    /// Sends one message. A failed send the reader will not dial again after
    /// marks the connection closed.
    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        self.shared.lock().send(msg).inspect_err(|_| {
            self.shared.state.store(CLOSED, Ordering::Relaxed);
        })
    }
}

#[cfg(test)]
mod tests;
