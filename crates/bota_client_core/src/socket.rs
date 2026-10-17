//! The two socket [`Transport`]s: the blocking [`Link`] with timeouts, and a
//! threaded [`Net`] whose poll never blocks the caller.
#![allow(clippy::disallowed_types)] // the connect deadline and receive timing read the clock

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{ClientMsg, FrameReader, ServerMsg, encode_frame_to_vec};

use crate::transport::Transport;

const READ_BUFFER_LEN: usize = 64 * 1024;
const MAX_RESOLVED_ADDRESSES: usize = 16;

/// One framed TCP connection that blocks on every read, up to a timeout.
pub struct Link {
    stream: TcpStream,
    reader: FrameReader,
    buffer: Box<[u8]>,
    receive_wait: Duration,
}

impl Link {
    /// Connects with one timeout for establishing and using the connection.
    pub fn connect(address: &str, timeout: Duration) -> std::io::Result<Self> {
        if timeout.is_zero() {
            return Err(std::io::Error::other("server I/O timeout must be positive"));
        }
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
        Ok(Self {
            stream,
            reader: FrameReader::new(),
            buffer: vec![0; READ_BUFFER_LEN].into_boxed_slice(),
            receive_wait: Duration::ZERO,
        })
    }
}

impl Transport for Link {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.receive_wait = Duration::ZERO;
        loop {
            match self.reader.next_message::<ServerMsg>() {
                Ok(Some(message)) => return Ok(Some(message)),
                Ok(None) => {}
                Err(error) => {
                    return Err(std::io::Error::other(format!(
                        "failed to decode server message: {error}; likely a protocol mismatch, \
build client and server from matching commits"
                    )));
                }
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

    fn take_receive_wait(&mut self) -> Option<Duration> {
        Some(std::mem::take(&mut self.receive_wait))
    }

    fn send(&mut self, message: &ClientMsg) -> std::io::Result<()> {
        let frame = encode_frame_to_vec(message).map_err(|error| {
            std::io::Error::other(format!("failed to encode client message: {error}"))
        })?;
        self.stream
            .write_all(&frame)
            .map_err(|error| io_context("failed to send client message", error))
    }
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

fn io_context(context: &str, error: std::io::Error) -> std::io::Error {
    std::io::Error::new(error.kind(), format!("{context}: {error}"))
}

/// A socket with a reader thread behind it.
///
/// Messages pile up in a channel; [`Net::poll`] hands over whatever has
/// arrived without ever blocking the render loop, and [`Transport::recv`]
/// waits for the next one.
pub struct Net {
    stream: TcpStream,
    inbox: Receiver<ServerMsg>,
    closed: Arc<AtomicBool>,
    /// Set when the reader met bytes that decode to no message.
    broken: Arc<AtomicBool>,
}

impl Net {
    /// Connects and starts reading.
    pub fn connect(addr: &str) -> std::io::Result<Net> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;
        let reader_stream = stream.try_clone()?;
        let (tx, inbox) = channel();
        let closed = Arc::new(AtomicBool::new(false));
        let closed_flag = Arc::clone(&closed);
        let broken = Arc::new(AtomicBool::new(false));
        let broken_flag = Arc::clone(&broken);
        thread::spawn(move || {
            let mut reader = FrameReader::new();
            let mut stream = reader_stream;
            let mut buf = [0u8; READ_BUFFER_LEN];
            loop {
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                reader.push(&buf[..n]);
                loop {
                    match reader.next_message::<ServerMsg>() {
                        Ok(Some(msg)) => {
                            if tx.send(msg).is_err() {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => {
                            broken_flag.store(true, Ordering::Relaxed);
                            closed_flag.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                }
            }
            closed_flag.store(true, Ordering::Relaxed);
        });
        Ok(Net {
            stream,
            inbox,
            closed,
            broken,
        })
    }

    /// Everything that arrived since the last poll.
    pub fn poll(&mut self) -> Vec<ServerMsg> {
        self.inbox.try_iter().collect()
    }

    /// Whether the server is gone.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

impl Transport for Net {
    /// Waits for the next message; none once the server closed cleanly.
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        match self.inbox.recv() {
            Ok(message) => Ok(Some(message)),
            Err(_) if self.broken.load(Ordering::Relaxed) => Err(std::io::Error::other(
                "failed to decode server message; likely a protocol mismatch, build client \
and server from matching commits",
            )),
            Err(_) => Ok(None),
        }
    }

    /// Sends one message. A failed send also marks the connection closed.
    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        let frame = encode_frame_to_vec(msg).map_err(|error| {
            std::io::Error::other(format!("failed to encode client message: {error}"))
        })?;
        self.stream.write_all(&frame).map_err(|error| {
            self.closed.store(true, Ordering::Relaxed);
            io_context("failed to send client message", error)
        })
    }
}
