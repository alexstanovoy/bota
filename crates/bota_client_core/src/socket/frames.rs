//! What the socket transports are made of: framed connections that read
//! whole messages and write whole frames, whatever carries them, and the
//! [`Endpoint`] that says which carrier to dial.

use std::io::ErrorKind;
use std::time::Duration;

use bota_proto::{ClientMsg, CodecError, ServerMsg, encode_frame_to_vec};

use super::tcp::TcpFrames;
#[cfg(feature = "websocket")]
use super::ws::{TlsConfig, WsFrames};

/// The reading half of a framed connection.
pub(crate) trait FrameSource: Send {
    /// The next server message, blocking up to the connection's I/O timeout;
    /// none after a clean close. A timeout, a reset or an undecodable
    /// message is an error (`InvalidData` for the last).
    fn read_message(&mut self) -> std::io::Result<Option<ServerMsg>>;

    /// Time spent waiting on the socket since the last call.
    fn take_wait(&mut self) -> Duration {
        Duration::ZERO
    }
}

/// The writing half of a framed connection.
pub(crate) trait FrameSink: Send {
    /// Sends whole frames (`u32 LE length | postcard`), one after another.
    fn send_frames(&mut self, frames: &[Vec<u8>]) -> std::io::Result<()>;

    /// Breaks the connection so that a reader blocked on it wakes with an
    /// error or the end.
    fn abort(&mut self);
}

/// A framed connection to a server, whole. [`Link`](super::Link) uses it as
/// it is; [`Net`](super::Net) splits it into a reader thread's half and the
/// caller's half.
pub(crate) trait FrameStream: FrameSource + FrameSink {
    fn split(self: Box<Self>) -> std::io::Result<(Box<dyn FrameSource>, Box<dyn FrameSink>)>;
}

/// Encodes `message` and sends it as one frame.
pub(crate) fn send_message(
    sink: &mut (impl FrameSink + ?Sized),
    message: &ClientMsg,
) -> std::io::Result<()> {
    let frame = encode_frame_to_vec(message).map_err(|error| {
        std::io::Error::other(format!("failed to encode client message: {error}"))
    })?;
    sink.send_frames(&[frame])
        .map_err(|error| io_context("failed to send client message", error))
}

pub(crate) fn undecodable(error: CodecError) -> std::io::Error {
    std::io::Error::new(
        ErrorKind::InvalidData,
        format!(
            "failed to decode server message: {error}; likely a protocol mismatch, build client \
and server from matching commits"
        ),
    )
}

pub(crate) fn io_context(context: &str, error: std::io::Error) -> std::io::Error {
    std::io::Error::new(error.kind(), format!("{context}: {error}"))
}

/// Where a client dials: a TCP address, or with the `websocket` feature a
/// `ws://` or `wss://` URL. Text parses as `host:port` or `tcp://host:port`
/// for TCP, which stays the default, and by its scheme otherwise.
#[derive(Clone)]
pub struct Endpoint {
    kind: Kind,
}

#[derive(Clone)]
enum Kind {
    Tcp(String),
    #[cfg(feature = "websocket")]
    Ws {
        url: String,
        tls: TlsConfig,
    },
}

impl Endpoint {
    pub fn parse(text: &str) -> std::io::Result<Endpoint> {
        if text.starts_with("ws://") || text.starts_with("wss://") {
            return Self::websocket(text);
        }
        Ok(Endpoint {
            kind: Kind::Tcp(text.strip_prefix("tcp://").unwrap_or(text).to_owned()),
        })
    }

    #[cfg(feature = "websocket")]
    fn websocket(url: &str) -> std::io::Result<Endpoint> {
        Ok(Endpoint {
            kind: Kind::Ws {
                url: url.to_owned(),
                tls: TlsConfig::webpki(),
            },
        })
    }

    #[cfg(not(feature = "websocket"))]
    fn websocket(url: &str) -> std::io::Result<Endpoint> {
        Err(std::io::Error::new(
            ErrorKind::Unsupported,
            format!("{url}: this build has no WebSocket transport (feature `websocket`)"),
        ))
    }

    /// The same endpoint trusting `tls` for a `wss://` server; TCP and
    /// `ws://` are not affected.
    #[cfg(feature = "websocket")]
    pub fn with_tls(mut self, tls: TlsConfig) -> Endpoint {
        if let Kind::Ws { tls: own, .. } = &mut self.kind {
            *own = tls;
        }
        self
    }

    /// Dials within `connect_timeout`; reads and writes are then bounded by
    /// `io_timeout`.
    pub(crate) fn open(
        &self,
        connect_timeout: Duration,
        io_timeout: Duration,
    ) -> std::io::Result<Box<dyn FrameStream>> {
        match &self.kind {
            Kind::Tcp(address) => Ok(Box::new(TcpFrames::open(
                address,
                connect_timeout,
                io_timeout,
            )?)),
            #[cfg(feature = "websocket")]
            Kind::Ws { url, tls } => Ok(Box::new(WsFrames::open(
                url,
                tls,
                connect_timeout,
                io_timeout,
            )?)),
        }
    }
}
