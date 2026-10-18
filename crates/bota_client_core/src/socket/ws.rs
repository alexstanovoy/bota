//! The WebSocket carrier: one binary message is one bota frame, exactly the
//! bytes a TCP stream would carry for it.

use std::io::{BufReader, ErrorKind};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use bota_proto::{LEN_PREFIX, MAX_PAYLOAD_LEN, ServerMsg};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};

use super::frames::{FrameSink, FrameSource, FrameStream, io_context};
use super::tcp::open_tcp;

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

/// How often a threaded reader looks at its outgoing queue.
const POLL: Duration = Duration::from_millis(1);
/// Frames waiting to go out on a threaded connection at most.
const MAX_OUTGOING: usize = 4096;

/// Which certificates a `wss://` client trusts: the public web roots, or the
/// certificate of a development server.
#[derive(Clone)]
pub struct TlsConfig {
    config: Arc<rustls::ClientConfig>,
}

impl TlsConfig {
    /// Trusts the certificates the public web does.
    pub fn webpki() -> TlsConfig {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        Self::with_roots(roots)
    }

    /// Trusts exactly the certificates in `pem`: a development server's own.
    pub fn trust_pem(pem: &str) -> std::io::Result<TlsConfig> {
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_reader_iter(&mut BufReader::new(pem.as_bytes())) {
            let cert = cert.map_err(std::io::Error::other)?;
            roots.add(cert).map_err(std::io::Error::other)?;
        }
        if roots.is_empty() {
            return Err(std::io::Error::other("no certificate in the pem"));
        }
        Ok(Self::with_roots(roots))
    }

    fn with_roots(roots: rustls::RootCertStore) -> TlsConfig {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("TLS 1.3 is supported by the ring provider")
            .with_root_certificates(roots)
            .with_no_client_auth();
        TlsConfig {
            config: Arc::new(config),
        }
    }
}

/// A WebSocket connection to a server, whole.
pub(crate) struct WsFrames {
    ws: Ws,
    wait: Duration,
}

impl WsFrames {
    pub fn open(
        url: &str,
        tls: &TlsConfig,
        connect_timeout: Duration,
        io_timeout: Duration,
    ) -> std::io::Result<Self> {
        let bad_url = || std::io::Error::new(ErrorKind::InvalidInput, format!("bad URL {url}"));
        let request = url.into_client_request().map_err(|_| bad_url())?;
        let uri = request.uri();
        let port = match (uri.port_u16(), uri.scheme_str()) {
            (Some(port), _) => port,
            (None, Some("wss")) => 443,
            (None, Some("ws")) => 80,
            _ => return Err(bad_url()),
        };
        let host = uri.host().ok_or_else(bad_url)?;
        let stream = open_tcp(&format!("{host}:{port}"), connect_timeout, io_timeout)?;
        let max = MAX_PAYLOAD_LEN + LEN_PREFIX;
        let config = WebSocketConfig::default()
            .max_message_size(Some(max))
            .max_frame_size(Some(max));
        let (ws, _) = tungstenite::client_tls_with_config(
            request,
            stream,
            Some(config),
            Some(Connector::Rustls(Arc::clone(&tls.config))),
        )
        .map_err(|error| {
            io_context(
                &format!("failed to open the WebSocket {url}"),
                match error {
                    tungstenite::HandshakeError::Failure(error) => ws_error(error),
                    tungstenite::HandshakeError::Interrupted(_) => {
                        std::io::Error::new(ErrorKind::TimedOut, "the handshake was interrupted")
                    }
                },
            )
        })?;
        Ok(Self {
            ws,
            wait: Duration::ZERO,
        })
    }
}

/// The socket under a connection, to set its timeouts.
fn socket(ws: &Ws) -> Option<&TcpStream> {
    match ws.get_ref() {
        MaybeTlsStream::Plain(stream) => Some(stream),
        MaybeTlsStream::Rustls(stream) => Some(stream.get_ref()),
        _ => None,
    }
}

fn ws_error(error: tungstenite::Error) -> std::io::Error {
    use tungstenite::Error;
    use tungstenite::error::ProtocolError;
    match error {
        Error::Io(error) => error,
        Error::ConnectionClosed | Error::AlreadyClosed => {
            std::io::Error::new(ErrorKind::BrokenPipe, "the connection is closed")
        }
        Error::Protocol(ProtocolError::ResetWithoutClosingHandshake) => std::io::Error::new(
            ErrorKind::ConnectionReset,
            "the connection ended without a closing handshake",
        ),
        Error::Capacity(_) | Error::Utf8(_) | Error::Protocol(_) => {
            std::io::Error::new(ErrorKind::InvalidData, error.to_string())
        }
        other => std::io::Error::other(other.to_string()),
    }
}

/// What one WebSocket read ended with.
enum Read {
    Message(ServerMsg),
    /// Nothing to hand on yet (a WebSocket-level ping, pong).
    Nothing,
    Closed,
}

fn read_one(ws: &mut Ws) -> std::io::Result<Read> {
    use tungstenite::Error;
    match ws.read() {
        Ok(Message::Binary(bytes)) => crate::ws_frame::decode(&bytes).map(Read::Message),
        Ok(Message::Close(_)) | Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
            Ok(Read::Closed)
        }
        Ok(Message::Text(_)) => Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "the server sent a text message; bota speaks binary frames only",
        )),
        Ok(_) => Ok(Read::Nothing),
        Err(error) => Err(io_context("failed to read server message", ws_error(error))),
    }
}

fn write_frames(ws: &mut Ws, frames: impl Iterator<Item = Vec<u8>>) -> std::io::Result<()> {
    for frame in frames {
        ws.write(Message::Binary(frame.into())).map_err(ws_error)?;
    }
    ws.flush().map_err(ws_error)
}

impl FrameSource for WsFrames {
    fn read_message(&mut self) -> std::io::Result<Option<ServerMsg>> {
        loop {
            let started = Instant::now();
            let read = read_one(&mut self.ws);
            self.wait = self.wait.saturating_add(started.elapsed());
            match read? {
                Read::Message(message) => return Ok(Some(message)),
                Read::Closed => return Ok(None),
                Read::Nothing => {}
            }
        }
    }

    fn take_wait(&mut self) -> Duration {
        std::mem::take(&mut self.wait)
    }
}

impl FrameSink for WsFrames {
    fn send_frames(&mut self, frames: &[Vec<u8>]) -> std::io::Result<()> {
        write_frames(&mut self.ws, frames.iter().cloned())
    }

    fn abort(&mut self) {
        if let Some(stream) = socket(&self.ws) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl FrameStream for WsFrames {
    /// A WebSocket cannot be read and written from two threads, so the
    /// reading half owns it and writes whatever the sending half queued,
    /// between reads that wait [`POLL`] at most.
    fn split(self: Box<Self>) -> std::io::Result<(Box<dyn FrameSource>, Box<dyn FrameSink>)> {
        let io_timeout = socket(&self.ws)
            .and_then(|stream| stream.read_timeout().ok().flatten())
            .unwrap_or(crate::transport::DEFAULT_IO_TIMEOUT);
        if let Some(stream) = socket(&self.ws) {
            stream.set_read_timeout(Some(POLL))?;
        }
        let (queue, outgoing) = sync_channel(MAX_OUTGOING);
        let (aborted, ended) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        let source = WsSource {
            ws: self.ws,
            outgoing,
            aborted: Arc::clone(&aborted),
            ended: Arc::clone(&ended),
            io_timeout,
        };
        let sink = WsSink {
            queue,
            aborted,
            ended,
        };
        Ok((Box::new(source), Box::new(sink)))
    }
}

struct WsSource {
    ws: Ws,
    outgoing: Receiver<Vec<u8>>,
    aborted: Arc<AtomicBool>,
    /// The server closed the connection cleanly.
    ended: Arc<AtomicBool>,
    /// Silence from the server this long is a dead link.
    io_timeout: Duration,
}

impl FrameSource for WsSource {
    fn read_message(&mut self) -> std::io::Result<Option<ServerMsg>> {
        let mut heard = Instant::now();
        loop {
            if self.aborted.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let queued = std::iter::from_fn(|| match self.outgoing.try_recv() {
                Ok(frame) => Some(frame),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
            });
            write_frames(&mut self.ws, queued)
                .map_err(|error| io_context("failed to send client message", error))?;
            match read_one(&mut self.ws) {
                Ok(Read::Message(message)) => return Ok(Some(message)),
                Ok(Read::Closed) => {
                    self.ended.store(true, Ordering::Relaxed);
                    return Ok(None);
                }
                Ok(Read::Nothing) => heard = Instant::now(),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    if heard.elapsed() >= self.io_timeout {
                        return Err(error);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }
}

struct WsSink {
    queue: SyncSender<Vec<u8>>,
    aborted: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
}

impl FrameSink for WsSink {
    fn send_frames(&mut self, frames: &[Vec<u8>]) -> std::io::Result<()> {
        for frame in frames {
            self.queue
                .try_send(frame.clone())
                .map_err(|error| match error {
                    std::sync::mpsc::TrySendError::Full(_) => std::io::Error::new(
                        ErrorKind::TimedOut,
                        "the connection is not taking frames",
                    ),
                    std::sync::mpsc::TrySendError::Disconnected(_) => {
                        std::io::Error::new(ErrorKind::BrokenPipe, "the connection has ended")
                    }
                })?;
        }
        Ok(())
    }

    fn abort(&mut self) {
        self.aborted.store(true, Ordering::Relaxed);
    }
}
