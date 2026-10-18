//! What a connection's threads read frames from and write frames to: a TCP
//! stream, or a WebSocket served beside it. Everything above (Hello, seats,
//! pings, the idle timeout, coming back, refusals and the allowance) is the
//! same for both.

use std::fmt;
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

use bota_proto::{LEN_PREFIX, MAX_CLIENT_PAYLOAD_LEN};

/// The kernel send buffer of one connection, in bytes: about four Map2 snapshots (3.3-3.8 KB
/// each).
pub const SEND_BUFFER_BYTES: usize = 16 * 1024;

/// How a peer broke the protocol. It is heard from no more, and its
/// connection is closed rather than reset: dialing again would not mend it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breach {
    /// A frame announced more than a client may send: `len` bytes of payload
    /// over TCP, of message over a WebSocket.
    Oversized { len: usize },
    /// Bytes that are no frame: a WebSocket text message, or a binary one
    /// whose length prefix disagrees with its size.
    Unframed,
    /// A frame whose payload decodes to no message.
    Undecodable,
    /// More messages than the connection's allowance.
    Flood,
}

impl fmt::Display for Breach {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Breach::Oversized { len } => write!(
                f,
                "announced a frame of {len} bytes, more than a client may send"
            ),
            Breach::Unframed => write!(f, "sent bytes that are no frame"),
            Breach::Undecodable => write!(f, "sent an undecodable message"),
            Breach::Flood => write!(f, "sent more messages than its allowance"),
        }
    }
}

/// Why a peer will be read from no more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadEnd {
    /// It hung up, broke the connection or sent nothing for the idle timeout.
    Gone,
    /// It broke the protocol.
    Breach(Breach),
}

/// One peer's transport: whole frames in and out.
pub trait FrameStream: Send {
    /// Splits the transport for a connection's reader and writer threads,
    /// with `idle_timeout` on every read and every write, and hands back the
    /// TCP socket underneath, kept to abort the connection.
    fn split(self: Box<Self>, idle_timeout: Option<Duration>) -> io::Result<Split>;
}

/// A transport taken apart for one connection's threads.
pub struct Split {
    pub source: Box<dyn FrameSource>,
    pub sink: Box<dyn FrameSink>,
    /// The socket underneath: a linger of zero and a shut reading side on it
    /// abort the connection, whichever thread holds the halves.
    pub socket: TcpStream,
}

/// What a connection's reader thread reads.
pub trait FrameSource: Send {
    /// The next frame whole, length prefix included, its payload at most
    /// [`MAX_CLIENT_PAYLOAD_LEN`] bytes long: a longer one is refused on its
    /// announced length, before a byte of it is buffered.
    fn next_frame(&mut self) -> Result<Vec<u8>, ReadEnd>;
}

/// What a connection's writer thread writes.
pub trait FrameSink: Send {
    /// Sends whole frames, one or more back to back. Fails when the peer took
    /// nothing for the idle timeout or the connection broke.
    fn send_frames(&mut self, frames: Vec<u8>) -> io::Result<()>;
    /// Ends the sending side once what was sent has gone out; the peer
    /// reads to the end, and the reader goes on until the peer hangs up.
    fn hang_up(&mut self);
    /// Ends the reading side, so the connection's reader reports the peer
    /// gone.
    fn shut_read(&mut self);
}

impl FrameStream for TcpStream {
    fn split(self: Box<Self>, idle_timeout: Option<Duration>) -> io::Result<Split> {
        let stream = *self;
        // Either failure only costs latency: Nagle's delay, or the kernel's default buffer, with
        // which the peer lags longer before it is skipped ahead.
        let _ = stream.set_nodelay(true);
        let _ = socket2::SockRef::from(&stream).set_send_buffer_size(SEND_BUFFER_BYTES);
        stream.set_read_timeout(idle_timeout)?;
        stream.set_write_timeout(idle_timeout)?;
        Ok(Split {
            source: Box::new(TcpSource(BufReader::new(stream.try_clone()?))),
            sink: Box::new(stream.try_clone()?),
            socket: stream,
        })
    }
}

/// The reading half of a TCP connection.
struct TcpSource(BufReader<TcpStream>);

impl FrameSource for TcpSource {
    fn next_frame(&mut self) -> Result<Vec<u8>, ReadEnd> {
        let mut prefix = [0u8; LEN_PREFIX];
        self.0.read_exact(&mut prefix).map_err(|_| ReadEnd::Gone)?;
        let len = u32::from_le_bytes(prefix) as usize;
        if len > MAX_CLIENT_PAYLOAD_LEN {
            return Err(ReadEnd::Breach(Breach::Oversized { len }));
        }
        let mut frame = vec![0u8; LEN_PREFIX + len];
        frame[..LEN_PREFIX].copy_from_slice(&prefix);
        self.0
            .read_exact(&mut frame[LEN_PREFIX..])
            .map_err(|_| ReadEnd::Gone)?;
        Ok(frame)
    }
}

impl FrameSink for TcpStream {
    fn send_frames(&mut self, frames: Vec<u8>) -> io::Result<()> {
        self.write_all(&frames)
    }

    fn hang_up(&mut self) {
        let _ = self.shutdown(Shutdown::Write);
    }

    fn shut_read(&mut self) {
        let _ = self.shutdown(Shutdown::Read);
    }
}
