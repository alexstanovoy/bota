//! One connected peer: reader thread, writer thread, outbox.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

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
    outbox: Arc<Outbox>,
    /// The thread draining `outbox` onto the socket; taken once joined.
    writer: Option<thread::JoinHandle<()>>,
}

impl Connection {
    /// Adopts an accepted stream: spawns its reader and writer threads.
    pub fn spawn(id: PlayerId, stream: TcpStream, events: Sender<NetEvent>) -> Connection {
        let _ = stream.set_nodelay(true);
        // A failure leaves the kernel's default buffer: the peer then lags longer before it is
        // skipped ahead, nothing worse.
        let _ = socket2::SockRef::from(&stream).set_send_buffer_size(SEND_BUFFER_BYTES);
        let outbox = Outbox::new();
        let writer_outbox = Arc::clone(&outbox);
        let writer_stream = stream.try_clone().expect("cloning a tcp stream");
        let writer = thread::spawn(move || writer_outbox.run_writer(writer_stream));
        thread::spawn(move || read_loop(id, stream, events));
        Connection {
            id,
            outbox,
            writer: Some(writer),
        }
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
}

fn read_loop(id: PlayerId, mut stream: TcpStream, events: Sender<NetEvent>) {
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        reader.push(&buf[..n]);
        loop {
            match reader.next_message::<ClientMsg>() {
                Ok(Some(msg)) => {
                    if events.send(NetEvent::Msg(id, msg)).is_err() {
                        return;
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
    let _ = events.send(NetEvent::Disconnected(id));
}

/// The accept thread body: hands every incoming stream to the server thread.
pub fn accept_loop(listener: std::net::TcpListener, events: Sender<NetEvent>) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if events.send(NetEvent::Connected(stream)).is_err() {
                    return;
                }
            }
            Err(_) => continue,
        }
    }
}
