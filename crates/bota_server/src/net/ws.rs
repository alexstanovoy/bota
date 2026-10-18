//! The WebSocket listener: TLS (when configured) and the upgrade run on a
//! small tokio runtime of their own, then each connection's frames pass to
//! and from its reader and writer threads through bounded channels, so
//! everything above the transport is the TCP code's.
//!
//! One binary message is one frame, length prefix included. The request's
//! path and `Origin` are not looked at: a seat key authenticates, and no
//! cookie is ever read.

use std::future::Future;
use std::io;
use std::net::{Shutdown, TcpStream as StdTcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::Duration;

use bota_proto::{LEN_PREFIX, MAX_CLIENT_PAYLOAD_LEN, MAX_PAYLOAD_LEN};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::{Semaphore, mpsc};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::error::CapacityError;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Bytes, Error as WsError, Message};

use crate::net::{
    ACCEPT_RETRY, Breach, FrameSink, FrameSource, FrameStream, NetEvent, ReadEnd,
    SEND_BUFFER_BYTES, Split,
};

/// How long a peer has for its TLS handshake and WebSocket upgrade together.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);
/// Handshakes under way at once; later peers wait in the kernel's backlog.
const MAX_HANDSHAKES: usize = 64;
/// Frames read ahead of a connection's reader thread.
const INBOUND_FRAMES: usize = 16;
/// The largest message a client may send: one frame of the largest payload.
const MAX_CLIENT_MESSAGE: usize = LEN_PREFIX + MAX_CLIENT_PAYLOAD_LEN;
/// Bytes read from the socket at once.
const READ_BUFFER: usize = 8 * 1024;
/// Bytes gathered before a write to the socket; every message is flushed
/// anyway.
const WRITE_BUFFER: usize = 64 * 1024;
/// The most held unwritten: what is gathered, plus the largest frame with its
/// message header.
const MAX_WRITE_BUFFER: usize = WRITE_BUFFER + LEN_PREFIX + MAX_PAYLOAD_LEN + 16;

/// TLS of a WebSocket listener: version 1.3 only, no client certificates.
#[derive(Clone)]
pub struct Tls(Arc<rustls::ServerConfig>);

impl Tls {
    /// The certificate chain (leaf first) and the private key in two PEM
    /// files.
    pub fn from_pem_files(cert: &Path, key: &Path) -> io::Result<Tls> {
        let invalid = |path: &Path, error: String| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {error}", path.display()),
            )
        };
        let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert)
            .and_then(Iterator::collect)
            .map_err(|error| invalid(cert, error.to_string()))?;
        if chain.is_empty() {
            return Err(invalid(cert, "no certificate".to_owned()));
        }
        let key_der =
            PrivateKeyDer::from_pem_file(key).map_err(|error| invalid(key, error.to_string()))?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .and_then(|builder| {
                builder
                    .with_no_client_auth()
                    .with_single_cert(chain, key_der)
            })
            .map_err(|error| invalid(cert, error.to_string()))?;
        Ok(Tls(Arc::new(config)))
    }
}

/// A bound WebSocket listener, plain (`ws://`) or over TLS (`wss://`).
pub struct WsListener {
    listener: std::net::TcpListener,
    tls: Option<Tls>,
}

impl WsListener {
    /// Speaks TLS on `listener` when given it.
    pub fn new(listener: std::net::TcpListener, tls: Option<Tls>) -> WsListener {
        WsListener { listener, tls }
    }

    /// Starts accepting on a thread of its own, whose runtime then carries
    /// every WebSocket connection's bytes; it ends once the server thread is
    /// gone and another peer connects, as the TCP accept thread does.
    pub(crate) fn start(self, events: std_mpsc::Sender<NetEvent>) -> io::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;
        self.listener.set_nonblocking(true)?;
        let listener = {
            let _entered = runtime.enter();
            TcpListener::from_std(self.listener)?
        };
        let tls = self.tls.map(|tls| TlsAcceptor::from(tls.0));
        thread::Builder::new()
            .spawn(move || runtime.block_on(accept_all(listener, tls, events)))?;
        Ok(())
    }
}

/// Accepts peers, at most [`MAX_HANDSHAKES`] of them in handshakes at once,
/// and hands each that completes in time to the server thread.
async fn accept_all(
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
    events: std_mpsc::Sender<NetEvent>,
) {
    let handshakes = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    let server_gone = Arc::new(AtomicBool::new(false));
    while !server_gone.load(Ordering::Relaxed) {
        let Ok(permit) = Arc::clone(&handshakes).acquire_owned().await else {
            return;
        };
        let tcp = match listener.accept().await {
            Ok((tcp, _)) => tcp,
            Err(_) => {
                tokio::time::sleep(ACCEPT_RETRY).await;
                continue;
            }
        };
        let (tls, events, server_gone) = (tls.clone(), events.clone(), Arc::clone(&server_gone));
        tokio::spawn(async move {
            let opened = tokio::time::timeout(HANDSHAKE_DEADLINE, open(tcp, tls)).await;
            drop(permit);
            if let Ok(Ok(stream)) = opened
                && events.send(NetEvent::Connected(stream)).is_err()
            {
                server_gone.store(true, Ordering::Relaxed);
            }
        });
    }
}

/// The TLS handshake when there is TLS, then the WebSocket upgrade.
async fn open(tcp: TcpStream, tls: Option<TlsAcceptor>) -> io::Result<Box<dyn FrameStream>> {
    // Either failure only costs latency, as on the TCP port.
    let _ = tcp.set_nodelay(true);
    let socket = {
        let socket = socket2::SockRef::from(&tcp);
        let _ = socket.set_send_buffer_size(SEND_BUFFER_BYTES);
        StdTcpStream::from(socket.try_clone()?)
    };
    match tls {
        None => upgrade(tcp, socket).await,
        Some(tls) => upgrade(tls.accept(tcp).await?, socket).await,
    }
}

async fn upgrade<S>(stream: S, socket: StdTcpStream) -> io::Result<Box<dyn FrameStream>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let config = WebSocketConfig::default()
        .read_buffer_size(READ_BUFFER)
        .write_buffer_size(WRITE_BUFFER)
        .max_write_buffer_size(MAX_WRITE_BUFFER)
        .max_message_size(Some(MAX_CLIENT_MESSAGE))
        .max_frame_size(Some(MAX_CLIENT_MESSAGE));
    let ws = tokio_tungstenite::accept_async_with_config(stream, Some(config))
        .await
        .map_err(io::Error::other)?;
    Ok(Box::new(WsStream {
        ws,
        socket,
        runtime: Handle::current(),
    }))
}

/// An upgraded connection, not yet read from.
struct WsStream<S> {
    ws: WebSocketStream<S>,
    /// The TCP socket underneath, shared with the runtime.
    socket: StdTcpStream,
    runtime: Handle,
}

impl<S> FrameStream for WsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    fn split(self: Box<Self>, idle_timeout: Option<Duration>) -> io::Result<Split> {
        let WsStream {
            ws,
            socket,
            runtime,
        } = *self;
        let (sink_socket, task_socket) = (socket.try_clone()?, socket.try_clone()?);
        let (inbound, frames) = mpsc::channel(INBOUND_FRAMES);
        let (outbound, queued) = mpsc::channel(1);
        let (done, finished) = std_mpsc::channel();
        let (sink, stream) = ws.split();
        runtime.spawn(read_side(stream, inbound, idle_timeout));
        runtime.spawn(write_side(sink, queued, idle_timeout, task_socket, done));
        Ok(Split {
            source: Box::new(WsSource(frames)),
            sink: Box::new(WsSink {
                outbound,
                finished,
                socket: sink_socket,
            }),
            socket,
        })
    }
}

/// What a connection's reader thread is handed: a frame, or why no more come.
type Inbound = Result<Vec<u8>, ReadEnd>;

/// Reads messages until the peer is gone, breaks the protocol or stays
/// silent past the idle timeout, passing frames on to the reader thread.
async fn read_side<S>(
    mut stream: SplitStream<WebSocketStream<S>>,
    inbound: mpsc::Sender<Inbound>,
    idle_timeout: Option<Duration>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let read = match within(idle_timeout, stream.next()).await {
            None => Err(ReadEnd::Gone),
            Some(next) => match frame_of(next) {
                Ok(None) => continue,
                Ok(Some(frame)) => Ok(frame),
                Err(end) => Err(end),
            },
        };
        let last = read.is_err();
        if inbound.send(read).await.is_err() || last {
            return;
        }
    }
}

/// What one read brings the reader thread: a frame, nothing (a control
/// message, answered by the WebSocket layer), or the end.
fn frame_of(next: Option<Result<Message, WsError>>) -> Result<Option<Vec<u8>>, ReadEnd> {
    let message = match next {
        Some(Ok(message)) => message,
        Some(Err(WsError::Capacity(CapacityError::MessageTooLong { size, .. }))) => {
            return Err(ReadEnd::Breach(Breach::Oversized { len: size }));
        }
        Some(Err(_)) | None => return Err(ReadEnd::Gone),
    };
    match message {
        Message::Binary(bytes) => whole_frame(bytes)
            .map(Some)
            .ok_or(ReadEnd::Breach(Breach::Unframed)),
        Message::Text(_) => Err(ReadEnd::Breach(Breach::Unframed)),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Ok(None),
        Message::Close(_) => Err(ReadEnd::Gone),
    }
}

/// A binary message as a frame, when its length prefix announces exactly
/// the rest of it.
fn whole_frame(bytes: Bytes) -> Option<Vec<u8>> {
    let prefix = bytes.first_chunk::<LEN_PREFIX>()?;
    let whole = u32::from_le_bytes(*prefix) as usize == bytes.len() - LEN_PREFIX;
    debug_assert!(bytes.len() <= MAX_CLIENT_MESSAGE, "the layer bounds it");
    whole.then(|| Vec::from(bytes))
}

/// What a connection's writer thread hands on.
enum Outbound {
    Frame(Bytes),
    /// Close the WebSocket once what came before is sent.
    HangUp,
}

/// Sends what the writer thread hands on, one frame per message. A frame the
/// peer takes no whole of within the idle timeout breaks the connection:
/// the socket's reading side is shut, so the reader reports the peer gone.
/// `done` is dropped once the task ends.
async fn write_side<S>(
    mut sink: SplitSink<WebSocketStream<S>, Message>,
    mut queued: mpsc::Receiver<Outbound>,
    idle_timeout: Option<Duration>,
    socket: StdTcpStream,
    done: std_mpsc::Sender<()>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    while let Some(next) = queued.recv().await {
        let frame = match next {
            Outbound::Frame(frame) => frame,
            Outbound::HangUp => {
                let _ = within(idle_timeout, sink.close()).await;
                break;
            }
        };
        let sent = within(idle_timeout, sink.send(Message::Binary(frame))).await;
        if !matches!(sent, Some(Ok(()))) {
            let _ = socket.shutdown(Shutdown::Read);
            break;
        }
    }
    drop(done);
}

/// `future`, given at most `limit`; nothing when it ran out.
async fn within<F: Future>(limit: Option<Duration>, future: F) -> Option<F::Output> {
    match limit {
        None => Some(future.await),
        Some(limit) => tokio::time::timeout(limit, future).await.ok(),
    }
}

/// The reader thread's end of a WebSocket connection.
struct WsSource(mpsc::Receiver<Inbound>);

impl FrameSource for WsSource {
    fn next_frame(&mut self) -> Result<Vec<u8>, ReadEnd> {
        self.0.blocking_recv().unwrap_or(Err(ReadEnd::Gone))
    }
}

/// The writer thread's end of a WebSocket connection.
struct WsSink {
    outbound: mpsc::Sender<Outbound>,
    /// Disconnected once the runtime's side has ended.
    finished: std_mpsc::Receiver<()>,
    socket: StdTcpStream,
}

impl FrameSink for WsSink {
    fn send_frames(&mut self, mut frames: Vec<u8>) -> io::Result<()> {
        while !frames.is_empty() {
            let rest = frames.split_off(frame_len(&frames));
            self.outbound
                .blocking_send(Outbound::Frame(Bytes::from(frames)))
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
            frames = rest;
        }
        Ok(())
    }

    /// Waits until the close is sent, as a TCP hang-up returns once its bytes
    /// are with the kernel.
    fn hang_up(&mut self) {
        if self.outbound.blocking_send(Outbound::HangUp).is_ok() {
            let _ = self.finished.recv();
        }
    }

    fn shut_read(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Read);
    }
}

/// The length of the frame at the front of `frames`, which the server
/// encoded whole.
fn frame_len(frames: &[u8]) -> usize {
    let prefix = frames
        .first_chunk::<LEN_PREFIX>()
        .expect("frames are whole");
    let len = LEN_PREFIX + u32::from_le_bytes(*prefix) as usize;
    assert!(len <= frames.len(), "frames are whole");
    len
}
