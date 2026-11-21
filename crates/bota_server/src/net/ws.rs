//! The WebSocket listener: TLS (when configured) and the upgrade run on a
//! small tokio runtime of their own, then each connection's frames pass to
//! and from its reader and writer threads through bounded channels, so
//! everything above the transport is the TCP code's.
//!
//! One binary message is one frame, length prefix included. The request's
//! path and `Origin` are not looked at: a seat key authenticates, and no
//! cookie is ever read. Nor is `X-Forwarded-For`, which any peer could forge:
//! behind a reverse proxy every peer counts as the proxy's address in the
//! bounds per source.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Shutdown, TcpStream as StdTcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bota_proto::{LEN_PREFIX, MAX_CLIENT_PAYLOAD_LEN, MAX_PAYLOAD_LEN, PlayerId};
use futures_util::future::{Either, select};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::error::CapacityError;
use tokio_tungstenite::tungstenite::protocol::{
    CloseFrame, WebSocketConfig, frame::coding::CloseCode,
};
use tokio_tungstenite::tungstenite::{Bytes, Error as WsError, Message};

use crate::net::{
    ACCEPT_RETRY, Breach, FrameSink, FrameSource, FrameStream, NetEvent, ReadEnd,
    SEND_BUFFER_BYTES, Split,
};

/// How long a peer has for its TLS handshake and WebSocket upgrade together.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);
/// Give the writer time to send the reason before disconnect handling can
/// discard its connection, even if a game frame is already queued.
const CLOSE_GRACE: Duration = Duration::from_secs(1);
/// How long the reader waits for the writer to take the idle notice while a game frame is in
/// the way. A writer that is not stuck frees its slot much sooner.
const IDLE_HANDOFF: Duration = Duration::from_millis(20);
/// Handshakes do not consume the adoption slots used by the TCP listener.
const MAX_HANDSHAKES: usize = 64;

/// Bound source tracking even when each upgrade comes from a different IP.
const MAX_LOG_SOURCES: usize = 64;
const PEER_LOG_LINES: u8 = 4;
const LOG_WINDOW: Duration = Duration::from_secs(1);

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
    #[cfg(test)]
    held_handshakes: Option<std_mpsc::Sender<()>>,
}

impl WsListener {
    /// Speaks TLS on `listener` when given it.
    pub fn new(listener: std::net::TcpListener, tls: Option<Tls>) -> WsListener {
        WsListener {
            listener,
            tls,
            #[cfg(test)]
            held_handshakes: None,
        }
    }

    /// The test owns handshake completion through its sockets, without racing the deadline.
    #[cfg(test)]
    pub(crate) fn hold_handshakes(mut self, started: std_mpsc::Sender<()>) -> Self {
        self.held_handshakes = Some(started);
        self
    }

    /// Starts accepting on a thread of its own, whose runtime then carries
    /// every WebSocket connection's bytes; it ends once the server thread is
    /// gone and another peer connects, as the TCP accept thread does.
    pub(super) fn start(
        self,
        events: std_mpsc::Sender<NetEvent>,
        pending: super::pending::PendingAdmissions,
    ) -> io::Result<()> {
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
        thread::Builder::new().spawn(move || {
            runtime.block_on(accept_all(
                listener,
                tls,
                events,
                pending,
                #[cfg(test)]
                self.held_handshakes,
            ))
        })?;
        Ok(())
    }
}

/// Accepts every peer as it comes, so that the kernel's backlog never fills, and hands each whose
/// handshake completes in time to the server thread. A peer is let go at once when
/// the handshake pool or its source's share is full. A completed handshake takes a separate
/// adoption reservation; stalled upgrades therefore cannot reserve the TCP listener's room.
async fn accept_all(
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
    events: std_mpsc::Sender<NetEvent>,
    pending: super::pending::PendingAdmissions,
    #[cfg(test)] held_handshakes: Option<std_mpsc::Sender<()>>,
) {
    let handshakes = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    let sources = super::Places::default();
    let server_gone = Arc::new(AtomicBool::new(false));
    let logs = Arc::new(Mutex::new(LogWindow::default()));
    let mut log_tick =
        tokio::time::interval_at(tokio::time::Instant::now() + LOG_WINDOW, LOG_WINDOW);
    log_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while !server_gone.load(Ordering::Relaxed) {
        let accepted = match select(
            std::pin::pin!(log_tick.tick()),
            std::pin::pin!(listener.accept()),
        )
        .await
        {
            Either::Left(_) => {
                report_suppressed(&logs, |line| eprintln!("{line}"));
                continue;
            }
            Either::Right((accepted, _)) => accepted,
        };
        let (tcp, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(_) => {
                tokio::time::sleep(ACCEPT_RETRY).await;
                continue;
            }
        };
        let Some(source) = sources.take(peer.ip()) else {
            continue;
        };
        let Ok(handshake) = Arc::clone(&handshakes).try_acquire_owned() else {
            continue;
        };
        let (tls, events, server_gone) = (tls.clone(), events.clone(), Arc::clone(&server_gone));
        let pending = pending.clone();
        let logs = Arc::clone(&logs);
        #[cfg(test)]
        let held_handshakes = held_handshakes.clone();
        tokio::spawn(async move {
            let opened = open_timed(
                tcp,
                tls,
                logs,
                #[cfg(test)]
                held_handshakes,
            )
            .await;
            drop((handshake, source));
            if let Ok(stream) = opened
                && let Some(admission) = pending.take(Some(peer.ip()))
                && events
                    .send(NetEvent::Connected(admission.stream(stream)))
                    .is_err()
            {
                server_gone.store(true, Ordering::Relaxed);
            }
        });
    }
}

async fn open_timed(
    tcp: TcpStream,
    tls: Option<TlsAcceptor>,
    logs: LogBudget,
    #[cfg(test)] held: Option<std_mpsc::Sender<()>>,
) -> io::Result<Box<dyn FrameStream>> {
    #[cfg(test)]
    if let Some(started) = held {
        let _ = started.send(());
        return open(tcp, tls, logs).await;
    }
    tokio::time::timeout(HANDSHAKE_DEADLINE, open(tcp, tls, logs))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "WebSocket handshake deadline"))?
}

/// The TLS handshake when there is TLS, then the WebSocket upgrade.
async fn open(
    tcp: TcpStream,
    tls: Option<TlsAcceptor>,
    logs: LogBudget,
) -> io::Result<Box<dyn FrameStream>> {
    // Either failure only costs latency, as on the TCP port.
    let _ = tcp.set_nodelay(true);
    let socket = {
        let socket = socket2::SockRef::from(&tcp);
        let _ = socket.set_send_buffer_size(SEND_BUFFER_BYTES);
        StdTcpStream::from(socket.try_clone()?)
    };
    match tls {
        None => upgrade(tcp, socket, logs).await,
        Some(tls) => upgrade(tls.accept(tcp).await?, socket, logs).await,
    }
}

async fn upgrade<S>(
    stream: S,
    socket: StdTcpStream,
    logs: LogBudget,
) -> io::Result<Box<dyn FrameStream>>
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
        logs,
    }))
}

/// An upgraded connection, not yet read from.
struct WsStream<S> {
    ws: WebSocketStream<S>,
    /// The TCP socket underneath, shared with the runtime.
    socket: StdTcpStream,
    runtime: Handle,
    logs: LogBudget,
}

impl<S> FrameStream for WsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    fn peer(&self) -> Option<IpAddr> {
        self.socket.peer_addr().ok().map(|addr| addr.ip())
    }

    fn split(self: Box<Self>, id: PlayerId, idle_timeout: Option<Duration>) -> io::Result<Split> {
        let (source, sink, socket) = self.spawn_tasks(id, idle_timeout)?;
        Ok(Split {
            source: Box::new(source),
            sink: Box::new(sink),
            socket,
        })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send + 'static> WsStream<S> {
    fn spawn_tasks(
        self,
        id: PlayerId,
        idle_timeout: Option<Duration>,
    ) -> io::Result<(WsSource, WsSink, StdTcpStream)> {
        let WsStream {
            ws,
            socket,
            runtime,
            logs,
        } = self;
        let log = Arc::new(SocketLog::new(
            id,
            socket.peer_addr().ok().map(|peer| peer.ip()),
            logs,
        ));
        let (sink_socket, task_socket) = (socket.try_clone()?, socket.try_clone()?);
        let (inbound, frames) = mpsc::channel(INBOUND_FRAMES);
        let (outbound, queued) = mpsc::channel(1);
        let (done, finished) = std_mpsc::channel();
        let (sink, stream) = ws.split();
        runtime.spawn(read_side(
            stream,
            inbound,
            outbound.clone(),
            idle_timeout,
            log.clone(),
        ));
        runtime.spawn(write_side(
            sink,
            queued,
            idle_timeout,
            task_socket,
            done,
            log,
        ));
        Ok((
            WsSource(frames),
            WsSink {
                outbound,
                finished,
                socket: sink_socket,
            },
            socket,
        ))
    }
}

type LogBudget = Arc<Mutex<LogWindow>>;

#[derive(Default)]
struct LogWindow {
    from_peer: HashMap<Option<IpAddr>, u8>,
    suppressed: u64,
}

impl LogWindow {
    fn admit_peer_end(&mut self, source: Option<IpAddr>) -> bool {
        if self.from_peer.len() >= MAX_LOG_SOURCES && !self.from_peer.contains_key(&source) {
            return false;
        }
        let used = self.from_peer.entry(source).or_default();
        if *used >= PEER_LOG_LINES {
            return false;
        }
        *used += 1;
        true
    }
}

fn report_suppressed(logs: &LogBudget, emit: impl FnOnce(std::fmt::Arguments<'_>)) {
    let suppressed = logs.lock().map_or(0, |mut window| {
        window.from_peer.clear();
        std::mem::take(&mut window.suppressed)
    });
    if suppressed != 0 {
        emit(format_args!(
            "bota_server: suppressed {suppressed} game WebSocket transport log lines"
        ));
    }
}

enum LogEnd {
    Peer,
    Failure,
}

struct SocketLog {
    id: PlayerId,
    source: Option<IpAddr>,
    budget: LogBudget,
    failure_reported: AtomicBool,
}

impl SocketLog {
    fn new(id: PlayerId, source: Option<IpAddr>, budget: LogBudget) -> Self {
        Self {
            id,
            source,
            budget,
            failure_reported: AtomicBool::new(false),
        }
    }

    fn ended(&self, end: LogEnd, reason: std::fmt::Arguments<'_>) {
        self.emit_end(end, reason, |line| eprintln!("{line}"));
    }

    fn emit_end(
        &self,
        end: LogEnd,
        reason: std::fmt::Arguments<'_>,
        emit: impl FnOnce(std::fmt::Arguments<'_>),
    ) {
        // Read and write can both fail as a connection dies. Its first
        // server-side diagnosis has its own slot, even on a noisy shared IP.
        let failure = match end {
            LogEnd::Failure => !self.failure_reported.swap(true, Ordering::Relaxed),
            LogEnd::Peer => false,
        };
        let allowed = failure
            || self.budget.lock().is_ok_and(|mut budget| {
                if matches!(end, LogEnd::Peer) && budget.admit_peer_end(self.source) {
                    true
                } else {
                    budget.suppressed = budget.suppressed.saturating_add(1);
                    false
                }
            });
        if allowed {
            emit(format_args!(
                "bota_server: game WebSocket connection {} {reason}",
                self.id.0
            ));
        }
    }
}

#[test]
fn noisy_websocket_closes_do_not_hide_another_connections_idle_reason() {
    let budget = Arc::new(Mutex::new(LogWindow::default()));
    let source = Some(IpAddr::from([127, 0, 0, 1]));
    let mut noise = 0;
    for id in 1..=16 {
        let log = SocketLog::new(PlayerId(id), source, Arc::clone(&budget));
        log.emit_end(LogEnd::Peer, format_args!("closed by its peer"), |_| {
            noise += 1
        });
    }
    assert_eq!(noise, usize::from(PEER_LOG_LINES));

    // The player shares an IP with the peer churning through connections.
    let honest = SocketLog::new(PlayerId(17), source, Arc::clone(&budget));
    let mut lines = Vec::new();
    honest.emit_end(
        LogEnd::Failure,
        format_args!("closed after going idle"),
        |line| lines.push(line.to_string()),
    );
    honest.emit_end(LogEnd::Failure, format_args!("write also failed"), |line| {
        lines.push(line.to_string())
    });
    assert_eq!(
        lines,
        ["bota_server: game WebSocket connection 17 closed after going idle"]
    );

    let elsewhere = SocketLog::new(
        PlayerId(18),
        Some(IpAddr::from([127, 0, 0, 2])),
        Arc::clone(&budget),
    );
    elsewhere.emit_end(LogEnd::Peer, format_args!("closed by its peer"), |line| {
        lines.push(line.to_string())
    });
    assert_eq!(
        lines.last().unwrap(),
        "bota_server: game WebSocket connection 18 closed by its peer"
    );

    let mut summary = None;
    report_suppressed(&budget, |line| summary = Some(line.to_string()));
    let suppressed = 16 - PEER_LOG_LINES + 1;
    assert_eq!(
        summary.unwrap(),
        format!("bota_server: suppressed {suppressed} game WebSocket transport log lines")
    );
}

#[test]
fn websocket_log_window_reports_suppression_and_bounds_source_tracking() {
    let budget = Arc::new(Mutex::new(LogWindow::default()));
    for index in 0..MAX_LOG_SOURCES {
        let mut window = budget.lock().unwrap();
        assert!(window.admit_peer_end(Some(IpAddr::from([
            127,
            0,
            0,
            u8::try_from(index).unwrap()
        ]))));
    }
    let extra = SocketLog::new(
        PlayerId(100),
        Some(IpAddr::from([127, 0, 0, 255])),
        Arc::clone(&budget),
    );
    extra.emit_end(LogEnd::Peer, format_args!("closed by its peer"), |_| {
        panic!("source table is full")
    });
    assert_eq!(budget.lock().unwrap().from_peer.len(), MAX_LOG_SOURCES);

    let mut summaries = Vec::new();
    report_suppressed(&budget, |line| summaries.push(line.to_string()));
    report_suppressed(&budget, |line| summaries.push(line.to_string()));
    assert_eq!(
        summaries,
        ["bota_server: suppressed 1 game WebSocket transport log lines"]
    );

    let mut allowed = false;
    extra.emit_end(LogEnd::Peer, format_args!("closed by its peer"), |_| {
        allowed = true
    });
    assert!(allowed, "the new window admits the source");
}

/// What a connection's reader thread is handed: a frame, or why no more come.
type Inbound = Result<Vec<u8>, ReadEnd>;

/// Reads messages until the peer is gone, breaks the protocol or stays
/// silent past the idle timeout, passing frames on to the reader thread.
async fn read_side<S>(
    mut stream: SplitStream<WebSocketStream<S>>,
    inbound: mpsc::Sender<Inbound>,
    outbound: mpsc::Sender<Outbound>,
    idle_timeout: Option<Duration>,
    log: Arc<SocketLog>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let read = match within(idle_timeout, stream.next()).await {
            None => {
                log.ended(
                    LogEnd::Failure,
                    format_args!("closed after {idle_timeout:?} without inbound traffic"),
                );
                announce_idle(&outbound).await;
                Err(ReadEnd::Gone)
            }
            Some(next) => match frame_of(next, &log) {
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

/// Tells the writer that the peer went idle and waits until the reason is sent, so the
/// disconnect that follows cannot discard it. Only a writer that takes the message at once is
/// waited for: one stuck on a line that takes nothing (a half-open connection) could not deliver
/// the reason anyway, and waiting would hold the seat past the idle timeout the peer's own
/// redial is timed against.
async fn announce_idle(outbound: &mpsc::Sender<Outbound>) {
    let (sent, received) = oneshot::channel();
    if matches!(
        within(Some(IDLE_HANDOFF), outbound.send(Outbound::Idle(sent))).await,
        Some(Ok(()))
    ) {
        let _ = within(Some(CLOSE_GRACE), received).await;
    }
}

/// What one read brings the reader thread: a frame, nothing (a control
/// message, answered by the WebSocket layer), or the end.
fn frame_of(
    next: Option<Result<Message, WsError>>,
    log: &SocketLog,
) -> Result<Option<Vec<u8>>, ReadEnd> {
    let message = match next {
        Some(Ok(message)) => message,
        Some(Err(WsError::Capacity(CapacityError::MessageTooLong { size, .. }))) => {
            return Err(ReadEnd::Breach(Breach::Oversized { len: size }));
        }
        Some(Err(error)) => {
            log.ended(LogEnd::Peer, format_args!("read failed: {error}"));
            return Err(ReadEnd::Gone);
        }
        None => {
            log.ended(LogEnd::Peer, format_args!("ended without a close frame"));
            return Err(ReadEnd::Gone);
        }
    };
    match message {
        Message::Binary(bytes) => whole_frame(bytes)
            .map(Some)
            .ok_or(ReadEnd::Breach(Breach::Unframed)),
        Message::Text(_) => Err(ReadEnd::Breach(Breach::Unframed)),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Ok(None),
        Message::Close(frame) => {
            let code = frame.as_ref().map(|frame| u16::from(frame.code));
            log.ended(
                LogEnd::Peer,
                format_args!("closed by its peer, status {code:?}"),
            );
            Err(ReadEnd::Gone)
        }
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
    /// The reader timed out. Acknowledge after flushing the reason so its
    /// disconnect event cannot race ahead and abort the writer.
    Idle(oneshot::Sender<()>),
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
    log: Arc<SocketLog>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut heartbeat = idle_timeout.map(|timeout| {
        let period = (timeout / 3).max(Duration::from_millis(10));
        let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        heartbeat
    });
    loop {
        // Chromium answers control pings in its network service even while
        // the page is busy; they test the transport, independently of the UI.
        let next = match select(
            std::pin::pin!(heartbeat_tick(&mut heartbeat)),
            std::pin::pin!(queued.recv()),
        )
        .await
        {
            Either::Right((next, _)) => next,
            Either::Left(((), _)) => {
                if send(&mut sink, Message::Ping(Bytes::new()), idle_timeout, &log).await {
                    continue;
                }
                let _ = socket.shutdown(Shutdown::Read);
                break;
            }
        };
        match next {
            Some(Outbound::Frame(frame)) => {
                if !send(&mut sink, Message::Binary(frame), idle_timeout, &log).await {
                    let _ = socket.shutdown(Shutdown::Read);
                    break;
                }
            }
            Some(Outbound::Idle(sent)) => {
                let close = Message::Close(Some(CloseFrame {
                    code: CloseCode::from(bota_proto::WEBSOCKET_IDLE_CLOSE),
                    reason: "game transport idle".into(),
                }));
                let _ = within(Some(CLOSE_GRACE), sink.send(close)).await;
                let _ = sent.send(());
                break;
            }
            Some(Outbound::HangUp) => {
                let _ = within(idle_timeout, sink.close()).await;
                break;
            }
            None => break,
        }
    }
    drop(done);
}

async fn heartbeat_tick(heartbeat: &mut Option<tokio::time::Interval>) {
    if let Some(timer) = heartbeat {
        timer.tick().await;
    } else {
        std::future::pending().await
    }
}

async fn send<S>(
    sink: &mut SplitSink<WebSocketStream<S>, Message>,
    message: Message,
    deadline: Option<Duration>,
    log: &SocketLog,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match within(deadline, sink.send(message)).await {
        Some(Ok(())) => true,
        Some(Err(error)) => {
            log.ended(LogEnd::Failure, format_args!("write failed: {error}"));
            false
        }
        None => {
            log.ended(
                LogEnd::Failure,
                format_args!("write timed out after {deadline:?}"),
            );
            false
        }
    }
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
    fn send_frames(&mut self, frames: Vec<u8>) -> io::Result<()> {
        let mut rest = frames.as_slice();
        while !rest.is_empty() {
            let (frame, tail) = rest.split_at(frame_len(rest));
            self.outbound
                .blocking_send(Outbound::Frame(Bytes::copy_from_slice(frame)))
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
            rest = tail;
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

#[cfg(test)]
#[test]
fn a_websocket_writer_ends_after_the_socket_is_aborted_without_an_idle_timer() {
    use tokio_tungstenite::tungstenite::client;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let peer = thread::spawn(move || {
        let socket = StdTcpStream::connect(address).expect("socket");
        client(format!("ws://{address}"), socket)
            .expect("upgrade")
            .0
    });
    let (socket, _) = listener.accept().expect("connected");
    socket.set_nonblocking(true).expect("asynchronous socket");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("runtime");
    let (mut source, sink, socket) = runtime.block_on(async {
        let tcp = TcpStream::from_std(socket.try_clone().expect("socket")).expect("async socket");
        let ws = tokio_tungstenite::accept_async(tcp).await.expect("upgrade");
        WsStream {
            ws,
            socket,
            runtime: Handle::current(),
            logs: Arc::new(Mutex::new(LogWindow::default())),
        }
        .spawn_tasks(PlayerId(1), None)
        .expect("transport tasks")
    });
    let peer = peer.join().expect("upgraded peer");
    // These are the two actions of Connection::abort and its outbox: shut
    // the read side and drop the writer's sender, leaving the peer open.
    socket.shutdown(Shutdown::Read).expect("abort socket");
    let WsSink {
        finished, outbound, ..
    } = sink;
    drop(outbound);
    thread::scope(|scope| {
        let (finish, done) = oneshot::channel();
        let waiter = scope.spawn(move || {
            let _ = finish.send(finished.recv_timeout(Duration::from_secs(5)));
        });
        assert_eq!(
            runtime.block_on(done).expect("writer result"),
            Err(std_mpsc::RecvTimeoutError::Disconnected)
        );
        waiter.join().expect("observer finished");
    });
    assert!(matches!(source.next_frame(), Err(ReadEnd::Gone)));
    drop(peer);
}
