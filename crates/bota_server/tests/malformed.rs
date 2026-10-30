//! A peer that breaks the protocol (bytes that decode to nothing, a frame
//! larger than a client may send, more messages than its allowance) is named
//! in the server's log and cut off alone, over TCP and, with the `websocket`
//! feature, over a WebSocket, plain and over TLS.

#[macro_use]
mod wire;

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bota_proto::{
    ClientMsg, FrameReader, MAX_CLIENT_PAYLOAD_LEN, Role, ServerMsg, SlotId, encode_frame_to_vec,
};
use wire::{Wire, dial};

/// How long a test waits for the server binary to speak or hang up. It guards against a hang
/// only: a busy machine makes starting a process and answering slow.
const WAIT: Duration = Duration::from_secs(300);

/// The running server binary, killed when the test ends.
struct Server {
    child: Child,
    /// Where a TCP client dials it over the wire under test.
    addr: String,
    log: mpsc::Receiver<String>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Lines of a pipe, forwarded as they come.
fn lines_of(pipe: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    rx
}

/// The listener arguments of a wire.
fn listener_args(wire: Wire) -> Vec<&'static str> {
    match wire {
        Wire::Tcp => vec!["--port", "0"],
        #[cfg(feature = "websocket")]
        Wire::Ws => vec!["--port", "0", "--ws-port", "0"],
        #[cfg(feature = "websocket")]
        Wire::Wss => vec![
            "--port",
            "0",
            "--ws-port",
            "0",
            "--tls-cert",
            wire::CERT,
            "--tls-key",
            wire::KEY,
        ],
    }
}

fn start(wire: Wire, args: &[&str]) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bota_server"))
        .args(listener_args(wire))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the server binary starts");
    let out = lines_of(child.stdout.take().expect("piped"));
    let log = lines_of(child.stderr.take().expect("piped"));
    // One line per listener, TCP first; the last speaks the wire.
    let lines = if wire == Wire::Tcp { 1 } else { 2 };
    let mut listening = String::new();
    for _ in 0..lines {
        listening = out.recv_timeout(WAIT).expect("the server announces itself");
    }
    let addr = listening
        .rsplit(['/', ' '])
        .next()
        .expect("an address ends the line");
    Server {
        child,
        addr: dial(wire, addr),
        log,
    }
}

over_every_wire!(
    a_peer_sending_what_cannot_be_decoded_is_logged_and_dropped_alone,
    a_frame_larger_than_a_client_may_send_is_refused_on_its_announced_length,
    a_flooding_peer_is_cut_off_alone,
);

fn connect(server: &Server) -> TcpStream {
    let stream = TcpStream::connect(&server.addr).expect("connect");
    stream.set_read_timeout(Some(WAIT)).expect("timeout");
    stream
}

fn send(stream: &mut TcpStream, msg: &ClientMsg) {
    let frame = encode_frame_to_vec(msg).expect("encode");
    stream.write_all(&frame).expect("send");
}

/// Waits for the welcome and returns the seat it names.
fn greet(stream: &mut TcpStream) -> Option<SlotId> {
    send(
        stream,
        &ClientMsg::hello(Role::Bot, "probe", None, bota_core::WIRE_FINGERPRINT),
    );
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 4096];
    loop {
        if let Some(ServerMsg::Welcome { slot, .. }) = reader
            .next_message::<ServerMsg>()
            .expect("the server speaks the protocol")
        {
            return slot;
        }
        let n = stream.read(&mut buf).expect("read");
        assert!(n > 0, "the server hung up before it welcomed");
        reader.push(&buf[..n]);
    }
}

fn a_peer_sending_what_cannot_be_decoded_is_logged_and_dropped_alone(wire: Wire) {
    let server = start(wire, &[]);
    let mut staying = connect(&server);
    assert_eq!(greet(&mut staying), Some(SlotId(0)));
    let mut leaving = connect(&server);
    assert_eq!(greet(&mut leaving), Some(SlotId(1)));

    // A whole frame whose payload names a message variant that does not exist.
    let payload = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(&payload);
    leaving.write_all(&frame).expect("send");

    let line = server
        .log
        .recv_timeout(WAIT)
        .expect("the failure is logged");
    assert!(line.contains("seat 1"), "the seat is named: {line}");
    assert!(
        line.contains("undecodable"),
        "and the failure is said: {line}"
    );

    let mut rest = Vec::new();
    leaving.read_to_end(&mut rest).expect("the server hangs up");

    // The other peer is still served, and the freed seat is handed out again.
    let mut again = connect(&server);
    assert_eq!(greet(&mut again), Some(SlotId(1)));
    send(&mut staying, &ClientMsg::SetReady(true));
}

/// Waits for the server to end a connection it cut off, by a close or a
/// reset: it may still hold bytes the peer sent past the cut.
fn assert_cut_off(stream: &mut TcpStream) {
    let mut rest = Vec::new();
    if let Err(error) = stream.read_to_end(&mut rest) {
        assert!(
            !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
            "the server kept the connection open"
        );
    }
}

fn a_frame_larger_than_a_client_may_send_is_refused_on_its_announced_length(wire: Wire) {
    let server = start(wire, &[]);
    let mut staying = connect(&server);
    assert_eq!(greet(&mut staying), Some(SlotId(0)));
    let mut leaving = connect(&server);
    assert_eq!(greet(&mut leaving), Some(SlotId(1)));

    let len = MAX_CLIENT_PAYLOAD_LEN + 1;
    leaving
        .write_all(&(len as u32).to_le_bytes())
        .expect("send");
    // Over TCP the prefix alone is refused, before any payload is waited for. A WebSocket
    // message announces its own length, so the adapter sends the frame whole.
    if wire != Wire::Tcp {
        leaving.write_all(&vec![0; len]).expect("send");
    }
    let line = server
        .log
        .recv_timeout(WAIT)
        .expect("the failure is logged");
    assert!(line.contains("seat 1"), "the seat is named: {line}");
    assert!(
        line.contains("announced a frame of"),
        "and the failure is said: {line}"
    );
    assert_cut_off(&mut leaving);

    let mut again = connect(&server);
    assert_eq!(greet(&mut again), Some(SlotId(1)));
    send(&mut staying, &ClientMsg::SetReady(true));
}

fn a_flooding_peer_is_cut_off_alone(wire: Wire) {
    // Twenty messages of burst; the hello and a pong a second stay well inside.
    let server = start(wire, &["--messages-per-sec", "10"]);
    let mut staying = connect(&server);
    assert_eq!(greet(&mut staying), Some(SlotId(0)));
    let mut leaving = connect(&server);
    assert_eq!(greet(&mut leaving), Some(SlotId(1)));

    let pong = encode_frame_to_vec(&ClientMsg::Pong).expect("encode");
    leaving.write_all(&pong.repeat(100)).expect("send");
    let line = server.log.recv_timeout(WAIT).expect("the flood is logged");
    assert!(line.contains("seat 1"), "the seat is named: {line}");
    assert!(
        line.contains("more messages than its allowance"),
        "and the failure is said: {line}"
    );
    assert_cut_off(&mut leaving);

    let mut again = connect(&server);
    assert_eq!(greet(&mut again), Some(SlotId(1)));
    send(&mut staying, &ClientMsg::SetReady(true));
}
