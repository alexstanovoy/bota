//! One connected peer: reader thread, writer thread, outbox.

use std::net::{Shutdown, TcpStream};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{
    ClientMsg, EventKind, LEN_PREFIX, PlayerId, ServerMsg, decode_payload, encode_frame_to_vec,
};

use crate::net::{Allowance, Breach, FrameSource, FrameStream, Outbox, ReadEnd, Split};

/// Everything that reaches the server thread from the network.
pub enum NetEvent {
    /// A peer connected: its TCP handshake, and on a WebSocket listener its
    /// TLS and upgrade, are done.
    Connected(Box<dyn FrameStream>),
    /// A peer sent a message.
    Msg(PlayerId, ClientMsg),
    /// A peer's connection ended, cleanly or not.
    Disconnected(PlayerId),
    /// A peer broke the protocol, and will be heard from no more.
    Breached(PlayerId, Breach),
}

/// The server-side handle of one peer.
pub struct Connection {
    /// Wire identity of the peer.
    pub id: PlayerId,
    /// When it was adopted, for the deadline of its Hello.
    pub opened: Instant,
    /// The socket underneath, kept to abort the connection from the server
    /// thread.
    socket: TcpStream,
    outbox: Arc<Outbox>,
    /// Ticks queued for the peer; each adds to its allowance.
    ticks_sent: Arc<AtomicU32>,
    /// The thread draining `outbox` onto the socket; taken once joined.
    writer: Option<thread::JoinHandle<()>>,
}

impl Connection {
    /// Adopts a connected peer: spawns its reader and writer threads. Fails
    /// when the system has no socket handle or thread left for it; the reader
    /// is spawned last, so a failed connection never reports an event.
    ///
    /// With an `idle_timeout`, a peer that sends nothing for that long, or
    /// takes nothing of what is sent for that long, is reported gone. A peer
    /// that sends more than `messages_per_sec` (see [`Allowance`]) is
    /// reported as a [`Breach::Flood`].
    pub fn spawn(
        id: PlayerId,
        stream: Box<dyn FrameStream>,
        events: Sender<NetEvent>,
        idle_timeout: Option<Duration>,
        messages_per_sec: NonZeroU32,
    ) -> std::io::Result<Connection> {
        let Split {
            source,
            sink,
            socket,
        } = stream.split(idle_timeout)?;
        let outbox = Outbox::new();
        let writer_outbox = Arc::clone(&outbox);
        let writer = thread::Builder::new().spawn(move || writer_outbox.run_writer(sink))?;
        let ticks_sent = Arc::new(AtomicU32::new(0));
        let allowance = Allowance::new(messages_per_sec, Arc::clone(&ticks_sent), Instant::now());
        let connection = Connection {
            id,
            opened: Instant::now(),
            socket,
            outbox,
            ticks_sent,
            writer: Some(writer),
        };
        if let Err(error) =
            thread::Builder::new().spawn(move || read_loop(id, source, events, allowance))
        {
            connection.abort();
            return Err(error);
        }
        Ok(connection)
    }

    /// Queues a message that must arrive.
    pub fn send(&self, msg: &ServerMsg) {
        if let Ok(frame) = encode_frame_to_vec(msg) {
            self.outbox.push(frame);
        }
    }

    /// Queues a line of chat, which never goes out compressed.
    pub fn send_chat(&self, msg: &ServerMsg) {
        if let Ok(frame) = encode_frame_to_vec(msg) {
            self.outbox.push_chat(frame);
        }
    }

    /// Compresses everything sent after this but chat; once is enough.
    pub fn compress(&self) {
        self.outbox.compress();
    }

    /// Queues a tick, its encoded snapshot frame and the events of it this
    /// peer may know, replacing any tick still waiting.
    pub fn send_tick(&self, snapshot: Vec<u8>, tick: u32, events: Vec<EventKind>) {
        self.ticks_sent.fetch_add(1, Ordering::Relaxed);
        self.outbox.push_tick(snapshot, tick, events);
    }

    /// Stops sending and waits for what is queued to reach the socket.
    pub fn close_and_wait(&mut self) {
        self.close();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }

    /// Stops sending and lets the writer drain and hang up.
    pub fn close(&self) {
        self.outbox.close();
    }

    /// Drops the connection of a peer that is gone or hopeless, without a
    /// word: what is queued is dropped, and the socket is reset rather than
    /// closed once the threads let go of it, so a client that is still there
    /// learns its connection broke (and may dial again) rather than that the
    /// server ended it.
    pub fn abort(&self) {
        let _ = socket2::SockRef::from(&self.socket).set_linger(Some(Duration::ZERO));
        self.outbox.abort();
        let _ = self.socket.shutdown(Shutdown::Read);
    }
}

/// Reads the peer's messages until it hangs up, breaks the connection, stays
/// silent past the idle timeout, breaks the protocol or spends its
/// allowance. A [`ClientMsg::Pong`] only shows the peer alive, which reading
/// it did.
///
/// Once the server stops listening, the reader still drains the connection
/// until the peer hangs up: a socket closed with bytes unread resets the
/// connection, and the peer would lose what it has not read yet, the end of
/// the match among it.
fn read_loop(
    id: PlayerId,
    mut source: Box<dyn FrameSource>,
    events: Sender<NetEvent>,
    mut allowance: Allowance,
) {
    let mut listened = true;
    let breach = loop {
        let frame = match source.next_frame() {
            Ok(frame) => frame,
            Err(ReadEnd::Gone) => break None,
            Err(ReadEnd::Breach(breach)) => break Some(breach),
        };
        if !listened {
            continue;
        }
        if !allowance.take(Instant::now()) {
            break Some(Breach::Flood);
        }
        match decode_payload::<ClientMsg>(&frame[LEN_PREFIX..]) {
            Ok(ClientMsg::Pong) => {}
            Ok(msg) => listened = events.send(NetEvent::Msg(id, msg)).is_ok(),
            Err(_) => break Some(Breach::Undecodable),
        }
    };
    if listened {
        let _ = events.send(match breach {
            None => NetEvent::Disconnected(id),
            Some(breach) => NetEvent::Breached(id, breach),
        });
    }
}

/// How long an accept loop rests after a failed accept, so a lasting
/// failure (no file handles left) does not spin it.
pub const ACCEPT_RETRY: Duration = Duration::from_millis(10);

/// The accept thread body: hands every incoming stream to the server thread.
pub fn accept_loop(listener: std::net::TcpListener, events: Sender<NetEvent>) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if events.send(NetEvent::Connected(Box::new(stream))).is_err() {
                    return;
                }
            }
            Err(_) => thread::sleep(ACCEPT_RETRY),
        }
    }
}
