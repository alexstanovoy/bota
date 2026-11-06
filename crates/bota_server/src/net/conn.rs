//! One connected peer: reader thread, writer thread, outbox.

use std::net::{IpAddr, Shutdown, TcpStream};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{
    ClientMsg, EventKind, LEN_PREFIX, PlayerId, ServerMsg, decode_payload, encode_frame_to_vec,
};

use crate::net::{
    Allowance, Breach, FrameSource, FrameStream, Linger, Outbox, Parting, ReadEnd, Split,
};

/// Everything that reaches the server thread from the network.
pub enum NetEvent {
    /// A peer connected: its TCP handshake, and on a WebSocket listener its
    /// TLS and upgrade, are done.
    Connected(Box<dyn FrameStream>),
    /// A peer sent a message.
    Msg(PlayerId, super::QueuedClientMessage),
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
    /// The address it comes from, which the bounds per source count ([`groups`]); none when
    /// the transport cannot tell.
    pub peer: Option<IpAddr>,
    /// The socket underneath, kept to abort the connection from the server
    /// thread.
    socket: TcpStream,
    outbox: Arc<Outbox>,
    /// Ticks queued for the peer; each adds to its allowance.
    ticks_sent: Arc<AtomicU32>,
}

/// How many connections hold threads: one counts from its adoption until its
/// reader and its writer have both ended, so a peer let go still counts while
/// its connection is read on.
#[derive(Clone, Default)]
pub struct LiveConnections(Arc<AtomicUsize>);

impl LiveConnections {
    /// How many connections hold threads now.
    pub fn count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    /// Counts one more connection until the [`Hold`] is dropped.
    fn hold(&self) -> Hold {
        self.0.fetch_add(1, Ordering::AcqRel);
        Hold(Arc::clone(&self.0))
    }
}

/// One connection's place among the live ones, given back when the last of
/// its threads lets go of it.
struct Hold(Arc<AtomicUsize>);

impl Drop for Hold {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// What every connection of a server is held to, and where it reports.
pub struct Terms<'a> {
    pub events: &'a Sender<NetEvent>,
    pub inbound: &'a super::InboundBudget,
    /// A peer that sends nothing for this long, or takes nothing of what is
    /// sent for this long, is reported gone.
    pub idle_timeout: Option<Duration>,
    /// A peer that sends more than this many messages a second (see
    /// [`Allowance`]) is reported as a [`Breach::Flood`].
    pub messages_per_sec: NonZeroU32,
    /// The tick the world is at, which the receipts of orders tell.
    pub world_tick: &'a Arc<AtomicU32>,
    /// The connections that hold threads.
    pub live: &'a LiveConnections,
    /// How long a peer let go is read on.
    pub linger: &'a Linger,
}

struct ReaderEvents {
    channel: Sender<NetEvent>,
    budget: Arc<super::inbound::PeerBudget>,
}

impl ReaderEvents {
    fn data(&self, id: PlayerId, message: ClientMsg) -> Result<bool, Breach> {
        let queued = self.budget.queue(message).ok_or(Breach::InputBacklog)?;
        Ok(self.channel.send(NetEvent::Msg(id, queued)).is_ok())
    }
}

impl Connection {
    /// Adopts a connected peer on the server's `terms`: spawns its reader and
    /// writer threads, which count among the live connections until both
    /// have ended. Fails when the system has no socket handle or thread left
    /// for it; the reader is spawned last, so a failed connection never
    /// reports an event.
    pub fn spawn(
        id: PlayerId,
        stream: Box<dyn FrameStream>,
        terms: &Terms<'_>,
    ) -> std::io::Result<Connection> {
        let reader_hold = Arc::new(terms.live.hold());
        let writer_hold = Arc::clone(&reader_hold);
        let peer = stream.peer();
        let Split {
            source,
            sink,
            socket,
        } = stream.split(terms.idle_timeout)?;
        let outbox = Outbox::new(terms.linger);
        let reader_outbox = Arc::clone(&outbox);
        let writer_outbox = Arc::clone(&outbox);
        thread::Builder::new().spawn(move || {
            let _held = writer_hold;
            writer_outbox.run_writer(sink);
        })?;
        let ticks_sent = Arc::new(AtomicU32::new(0));
        let allowance = Allowance::new(
            terms.messages_per_sec,
            Arc::clone(&ticks_sent),
            Instant::now(),
        );
        let events = ReaderEvents {
            channel: terms.events.clone(),
            budget: terms.inbound.peer(),
        };
        let world_tick = Arc::clone(terms.world_tick);
        let opened = Instant::now();
        let connection = Connection {
            id,
            opened,
            peer,
            socket,
            outbox,
            ticks_sent,
        };
        if let Err(error) = thread::Builder::new().spawn(move || {
            let _held = reader_hold;
            read_loop(
                id,
                source,
                events,
                allowance,
                &reader_outbox,
                opened,
                &world_tick,
            )
        }) {
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

    /// Ends the connection at the end of the match: waits for what is queued
    /// to reach the socket, and reads on until the peer hangs up.
    pub fn close_and_wait(&self) {
        self.outbox.close(Parting::MatchOver);
        self.outbox.wait_sent();
    }

    /// Lets the peer go: the writer drains what is queued and hangs up, and
    /// the connection is read on for a bounded while only.
    pub fn close(&self) {
        self.outbox.close(Parting::LetGo);
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
/// it did. A [`ClientMsg::Probe`] is answered here, on the spot, ahead of what
/// waits in the outbox: the simulation never sees it, so the peer times the
/// line and not the tick.
///
/// Data queue exhaustion stops this reader and reports a breach. The simulation consumes the
/// already queued prefix, then disconnects the sender; no order is silently dropped. Terminal
/// events are not charged to the data budget, so exhaustion cannot hide the disconnect.
///
/// Once the server stops listening, the reader still drains the connection
/// until the peer hangs up, or, for a peer let go, until the writer shuts the
/// reading side a while after its own hang-up: a socket closed with bytes
/// unread resets the connection, and the peer would lose what it has not
/// read yet, the end of the match among it.
fn read_loop(
    id: PlayerId,
    mut source: Box<dyn FrameSource>,
    events: ReaderEvents,
    mut allowance: Allowance,
    outbox: &Outbox,
    opened: Instant,
    world_tick: &AtomicU32,
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
            Ok(ClientMsg::Probe { id }) => {
                if let Ok(echo) = encode_frame_to_vec(&ServerMsg::Echo { id }) {
                    outbox.push_urgent(echo);
                }
            }
            Ok(msg) => {
                if let ClientMsg::Order { seq, .. } = msg {
                    let received = ServerMsg::OrderReceived {
                        seq,
                        tick: world_tick.load(Ordering::Relaxed).saturating_add(1),
                        at_us: u64::try_from(opened.elapsed().as_micros()).unwrap_or(u64::MAX),
                    };
                    if let Ok(frame) = encode_frame_to_vec(&received) {
                        outbox.push_urgent(frame);
                    }
                }
                match events.data(id, msg) {
                    Ok(accepted) => listened = accepted,
                    Err(breach) => break Some(breach),
                }
            }
            Err(_) => break Some(Breach::Undecodable),
        }
    };
    if listened {
        let _ = events.channel.send(match breach {
            None => NetEvent::Disconnected(id),
            Some(breach) => NetEvent::Breached(id, breach),
        });
    }
    outbox.reader_ended();
}

/// How long an accept loop rests after a failed accept, so a lasting
/// failure (no file handles left) does not spin it.
pub const ACCEPT_RETRY: Duration = Duration::from_millis(10);

pub(super) fn accept_pending(
    listener: std::net::TcpListener,
    events: Sender<NetEvent>,
    pending: super::pending::PendingAdmissions,
) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let Some(admission) = pending.take(stream.peer_addr().ok().map(|peer| peer.ip()))
                else {
                    continue;
                };
                if events
                    .send(NetEvent::Connected(admission.stream(Box::new(stream))))
                    .is_err()
                {
                    return;
                }
            }
            Err(_) => thread::sleep(ACCEPT_RETRY),
        }
    }
}
