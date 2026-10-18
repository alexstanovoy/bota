//! The TCP carrier: frames are the byte stream itself.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use bota_proto::{FrameReader, ServerMsg};

use super::frames::{FrameSink, FrameSource, FrameStream, io_context, undecodable};

const READ_BUFFER_LEN: usize = 64 * 1024;
const MAX_RESOLVED_ADDRESSES: usize = 16;

/// The reading side of a TCP connection, which also holds its stream and so
/// can stand for the whole connection.
pub(crate) struct TcpFrames {
    stream: TcpStream,
    reader: FrameReader,
    buffer: Box<[u8]>,
    wait: Duration,
}

impl TcpFrames {
    pub fn open(
        address: &str,
        connect_timeout: Duration,
        io_timeout: Duration,
    ) -> std::io::Result<Self> {
        Ok(Self::new(open_tcp(address, connect_timeout, io_timeout)?))
    }

    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            reader: FrameReader::new(),
            buffer: vec![0; READ_BUFFER_LEN].into_boxed_slice(),
            wait: Duration::ZERO,
        }
    }
}

impl FrameSource for TcpFrames {
    fn read_message(&mut self) -> std::io::Result<Option<ServerMsg>> {
        loop {
            if let Some(message) = self.reader.next_message().map_err(undecodable)? {
                return Ok(Some(message));
            }
            let started = Instant::now();
            let read = self.stream.read(&mut self.buffer);
            self.wait = self.wait.saturating_add(started.elapsed());
            let read = match read {
                Ok(0) => return Ok(None),
                Ok(read) => read,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(io_context("failed to read server message", error)),
            };
            self.reader.push(&self.buffer[..read]);
        }
    }

    fn take_wait(&mut self) -> Duration {
        std::mem::take(&mut self.wait)
    }
}

impl FrameSink for TcpStream {
    fn send_frames(&mut self, frames: &[Vec<u8>]) -> std::io::Result<()> {
        match frames {
            [one] => self.write_all(one),
            many => self.write_all(&many.concat()),
        }
    }

    fn abort(&mut self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

impl FrameSink for TcpFrames {
    fn send_frames(&mut self, frames: &[Vec<u8>]) -> std::io::Result<()> {
        self.stream.send_frames(frames)
    }

    fn abort(&mut self) {
        self.stream.abort();
    }
}

impl FrameStream for TcpFrames {
    fn split(self: Box<Self>) -> std::io::Result<(Box<dyn FrameSource>, Box<dyn FrameSink>)> {
        let reader = Self::new(self.stream.try_clone()?);
        Ok((Box::new(reader), Box::new(self.stream)))
    }
}

/// Connects within `connect_timeout` and sets the stream up for framed
/// traffic: no Nagle delay, reads and writes bounded by `io_timeout`.
pub fn open_tcp(
    address: &str,
    connect_timeout: Duration,
    io_timeout: Duration,
) -> std::io::Result<TcpStream> {
    let stream = connect(address, connect_timeout)?;
    stream
        .set_nodelay(true)
        .map_err(|error| io_context("failed to enable TCP_NODELAY", error))?;
    stream
        .set_read_timeout(Some(io_timeout))
        .map_err(|error| io_context("failed to set read timeout", error))?;
    stream
        .set_write_timeout(Some(io_timeout))
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
