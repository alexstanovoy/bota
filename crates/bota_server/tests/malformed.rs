//! A peer that speaks a protocol the server cannot decode is named in the
//! server's log and cut off alone.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bota_proto::{ClientMsg, FrameReader, Role, ServerMsg, SlotId, encode_frame_to_vec};

const WAIT: Duration = Duration::from_secs(10);

/// The running server binary, killed when the test ends.
struct Server {
    child: Child,
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

fn start() -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bota_server"))
        .args(["--port", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the server binary starts");
    let out = lines_of(child.stdout.take().expect("piped"));
    let log = lines_of(child.stderr.take().expect("piped"));
    let mut server = Server {
        child,
        addr: String::new(),
        log,
    };
    let listening = out.recv_timeout(WAIT).expect("the server announces itself");
    server.addr = listening
        .rsplit(' ')
        .next()
        .expect("an address ends the line")
        .to_string();
    server
}

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
        &ClientMsg::Hello {
            role: Role::Bot,
            name: "probe".to_string(),
            key: None,
        },
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

#[test]
fn a_peer_sending_what_cannot_be_decoded_is_logged_and_dropped_alone() {
    let server = start();
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
