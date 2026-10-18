//! One connected peer: reader thread, writer thread, outbox.

use std::io::{self, Read};
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{
    ClientMsg, CodecError, EventKind, FrameReader, PlayerId, ServerMsg, encode_frame_to_vec,
};

use crate::net::Outbox;

/// The kernel send buffer of one connection, in bytes: about four Map2 snapshots (3.3-3.8 KB
/// each).
pub const SEND_BUFFER_BYTES: usize = 16 * 1024;

/// Everything that reaches the server thread from the network.
pub enum NetEvent {
    /// A peer finished the TCP handshake.
    Connected(TcpStream),
    /// A peer sent a message.
    Msg(PlayerId, ClientMsg),
    /// A peer's connection ended, cleanly or not.
    Disconnected(PlayerId),
    /// A peer sent bytes that are no message of this protocol, and will
    /// be heard from no more.
    Malformed(PlayerId, CodecError),
}

/// The server-side handle of one peer.
pub struct Connection {
    /// Wire identity of the peer.
    pub id: PlayerId,
    /// When it was adopted, for the deadline of its Hello.
    pub opened: Instant,
    /// Kept to abort the connection from the server thread.
    stream: TcpStream,
    outbox: Arc<Outbox>,
    /// The thread draining `outbox` onto the socket; taken once joined.
    writer: Option<thread::JoinHandle<()>>,
}

impl Connection {
    /// Adopts an accepted stream: spawns its reader and writer threads. Fails
    /// when the system has no socket handle or thread left for it; the reader
    /// is spawned last, so a failed connection never reports an event.
    ///
    /// With an `idle_timeout`, a peer that sends nothing for that long, or
    /// takes nothing of what is sent for that long, is reported gone.
    pub fn spawn(
        id: PlayerId,
        stream: TcpStream,
        events: Sender<NetEvent>,
        idle_timeout: Option<Duration>,
    ) -> io::Result<Connection> {
        // Either failure only costs latency: Nagle's delay, or the kernel's default buffer, with
        // which the peer lags longer before it is skipped ahead.
        let _ = stream.set_nodelay(true);
        let _ = socket2::SockRef::from(&stream).set_send_buffer_size(SEND_BUFFER_BYTES);
        stream.set_read_timeout(idle_timeout)?;
        stream.set_write_timeout(idle_timeout)?;
        let outbox = Outbox::new();
        let writer_outbox = Arc::clone(&outbox);
        let writer_stream = stream.try_clone()?;
        let reader_stream = stream.try_clone()?;
        let writer =
            thread::Builder::new().spawn(move || writer_outbox.run_writer(writer_stream))?;
        let connection = Connection {
            id,
            opened: Instant::now(),
            stream,
            outbox,
            writer: Some(writer),
        };
        if let Err(error) =
            thread::Builder::new().spawn(move || read_loop(id, reader_stream, events))
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

    /// Queues a tick, its encoded snapshot frame and the events of it this
    /// peer may know, replacing any tick still waiting.
    pub fn send_tick(&self, snapshot: Vec<u8>, tick: u32, events: Vec<EventKind>) {
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
        let _ = socket2::SockRef::from(&self.stream).set_linger(Some(Duration::ZERO));
        self.outbox.abort();
        let _ = self.stream.shutdown(Shutdown::Read);
    }
}

/// Reads the peer's messages until it hangs up, breaks the connection, stays
/// silent past the read timeout or sends bytes that decode to nothing. A
/// [`ClientMsg::Pong`] only shows the peer alive, which reading it did.
///
/// Once the server stops listening, the reader still drains the socket until
/// the peer hangs up: a socket closed with bytes unread resets the
/// connection, and the peer would lose what it has not read yet, the end of
/// the match among it.
fn read_loop(id: PlayerId, mut stream: TcpStream, events: Sender<NetEvent>) {
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 4096];
    let mut listened = true;
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
            Ok(n) => n,
        };
        if !listened {
            continue;
        }
        reader.push(&buf[..n]);
        loop {
            match reader.next_message::<ClientMsg>() {
                Ok(Some(ClientMsg::Pong)) => {}
                Ok(Some(msg)) => {
                    if events.send(NetEvent::Msg(id, msg)).is_err() {
                        listened = false;
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = events.send(NetEvent::Malformed(id, error));
                    return;
                }
            }
        }
    }
    if listened {
        let _ = events.send(NetEvent::Disconnected(id));
    }
}

/// How long the accept thread rests after a failed accept, so a lasting
/// failure (no file handles left) does not spin it.
const ACCEPT_RETRY: Duration = Duration::from_millis(10);

/// The accept thread body: hands every incoming stream to the server thread.
pub fn accept_loop(listener: std::net::TcpListener, events: Sender<NetEvent>) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if events.send(NetEvent::Connected(stream)).is_err() {
                    return;
                }
            }
            Err(_) => thread::sleep(ACCEPT_RETRY),
        }
    }
}
