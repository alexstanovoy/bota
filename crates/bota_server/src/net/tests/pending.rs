//! Accepted sockets remain bounded while the simulation is not consuming its inbox.

use super::*;
use crate::net::{MAX_PENDING_CONNECTIONS, MAX_PER_IPV4, NetEvent};

const WAIT: Duration = Duration::from_secs(10);

fn connect_from(source: u8, port: u16, websocket: bool) -> TcpStream {
    let socket =
        socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).expect("a socket");
    socket
        .bind(&std::net::SocketAddr::from(([127, 0, 0, source], 0)).into())
        .expect("a source");
    socket
        .connect(&std::net::SocketAddr::from(([127, 0, 0, 1], port)).into())
        .expect("connect");
    let mut stream: TcpStream = socket.into();
    stream.set_read_timeout(Some(WAIT)).expect("a hang guard");
    if websocket {
        stream.write_all(super::limits::UPGRADE).expect("upgrade");
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") && head.len() < 4096 {
            let mut byte = [0];
            stream.read_exact(&mut byte).expect("the upgrade completes");
            head.push(byte[0]);
        }
        assert!(head.starts_with(b"HTTP/1.1 101"), "an upgrade response");
    }
    stream
}

fn listeners() -> (u16, u16, mpsc::Receiver<NetEvent>) {
    let tcp = TcpListener::bind("127.0.0.1:0").expect("TCP listener");
    let ws = TcpListener::bind("127.0.0.1:0").expect("WebSocket listener");
    let ports = (
        tcp.local_addr().unwrap().port(),
        ws.local_addr().unwrap().port(),
    );
    let listeners = Listeners::with_websocket(tcp, crate::net::WsListener::new(ws, None));
    let (send, recv) = mpsc::channel();
    listeners.start(&send).expect("listeners start");
    (ports.0, ports.1, recv)
}

fn rejected(stream: &mut TcpStream) {
    let mut byte = [0];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("an excess pending socket must close, not remain queued: {other:?}"),
    }
}

fn take_pending(recv: &mpsc::Receiver<NetEvent>) -> Box<dyn crate::net::FrameStream> {
    match recv.recv_timeout(WAIT).expect("an admitted stream") {
        NetEvent::Connected(stream) => stream,
        _ => panic!("only incoming connections are queued before adoption"),
    }
}

#[test]
fn pending_admissions_share_a_source_bound_across_tcp_and_websocket() {
    let (tcp, ws, recv) = listeners();
    let clients: Vec<_> = (0..MAX_PER_IPV4)
        .map(|n| connect_from(1, if n % 2 == 0 { tcp } else { ws }, n % 2 != 0))
        .collect();
    rejected(&mut connect_from(1, tcp, false));
    let mut excess = connect_from(1, ws, false);
    let _ = excess.write_all(super::limits::UPGRADE);
    rejected(&mut excess);
    // Taking a queued stream alone does not release its reservation: adoption still owns it.
    let pending = take_pending(&recv);
    rejected(&mut connect_from(1, tcp, false));
    drop(pending);
    let fresh = connect_from(1, ws, true);
    for _ in 0..MAX_PER_IPV4 {
        drop(take_pending(&recv));
    }
    assert!(
        recv.try_recv().is_err(),
        "no excess socket entered the queue"
    );
    drop((clients, fresh));
}

#[test]
fn pending_admissions_share_a_global_bound_across_tcp_and_websocket() {
    let (tcp, ws, recv) = listeners();
    let clients: Vec<_> = (0..MAX_PENDING_CONNECTIONS)
        .map(|n| connect_from(n as u8 + 1, if n % 2 == 0 { tcp } else { ws }, n % 2 != 0))
        .collect();
    rejected(&mut connect_from(100, tcp, false));
    for _ in 0..MAX_PENDING_CONNECTIONS {
        drop(take_pending(&recv));
    }
    let fresh = connect_from(100, ws, true);
    drop(take_pending(&recv));
    assert!(
        recv.try_recv().is_err(),
        "no excess socket entered the queue"
    );
    drop((clients, fresh));
}
