//! One connected peer: reader thread, writer thread, outbox.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

use bota_proto::{ClientMsg, FrameReader, PlayerId, ServerMsg, encode_frame_to_vec};

use crate::net::Outbox;

/// Everything that reaches the server thread from the network.
pub enum NetEvent {
    /// A peer finished the TCP handshake.
    Connected(TcpStream),
    /// A peer sent a message.
    Msg(PlayerId, ClientMsg),
    /// A peer's connection ended, cleanly or not.
    Disconnected(PlayerId),
}

/// The server-side handle of one peer.
pub struct Connection {
    /// Wire identity of the peer.
    pub id: PlayerId,
    /// Frames on their way out.
    pub outbox: Arc<Outbox>,
    /// The thread putting them on the socket, kept so the last of them can be
    /// waited for.
    writer: Option<thread::JoinHandle<()>>,
}

impl Connection {
    /// Adopts an accepted stream: spawns its reader and writer threads.
    pub fn spawn(id: PlayerId, stream: TcpStream, events: Sender<NetEvent>) -> Connection {
        let _ = stream.set_nodelay(true);
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

    /// Queues a snapshot frame, replacing any still-waiting one.
    pub fn send_snapshot(&self, frame: Vec<u8>) {
        self.outbox.push_snapshot(frame);
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
                Err(_) => {
                    let _ = events.send(NetEvent::Disconnected(id));
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
