//! What one source may hold of a server: its bounds of handshakes and of connections that have
//! not said Hello. However many it opens, a fair client from elsewhere is taken, a newcomer past
//! the strangers' share takes a stranger's place, never a connection's that said Hello, and
//! players behind one address all join.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::spectators::Spectators;

/// How long the fair client is given: a guard against the hang a flooded server caused, not a
/// measure of speed.
const FAIR_GUARD: Duration = Duration::from_secs(2);

/// A server listening on every address of both families, TCP and WebSocket: a client dials it
/// from 127.0.0.1 and from ::1, two sources.
fn start_on_both_families(opts: ServerOpts) -> (u16, u16) {
    let tcp = TcpListener::bind("[::]:0").expect("bind");
    let ws = TcpListener::bind("[::]:0").expect("bind");
    let ports = (
        tcp.local_addr().expect("addr").port(),
        ws.local_addr().expect("addr").port(),
    );
    let listeners = Listeners::with_websocket(tcp, crate::net::WsListener::new(ws, None));
    thread::spawn(move || run(listeners, opts));
    ports
}

/// What a flooding peer does with each connection it opens.
#[derive(Clone, Copy)]
enum Stall {
    /// Nothing: a TCP or TLS handshake that never goes on.
    Silent,
    /// The WebSocket upgrade, then nothing: a stranger that never says Hello.
    Upgraded,
}

const UPGRADE: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";

/// Threads that each hold a connection to `addr` open, doing `stall` on it, and open the next as
/// soon as the server lets it go, until the returned flag is raised.
fn flood(addr: SocketAddr, stall: Stall, threads: usize) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    for _ in 0..threads {
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
                else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                if let Stall::Upgraded = stall {
                    let _ = stream.write_all(UPGRADE);
                }
                let mut buf = [0u8; 4096];
                while !stop.load(Ordering::Relaxed) {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                        Err(_) => break,
                    }
                }
            }
        });
    }
    stop
}

/// One source floods every port with stalled handshakes and with strangers; a fair client from
/// another source still connects over a WebSocket and is welcomed.
#[test]
fn a_flooding_source_cannot_keep_a_fair_client_out() {
    let mut opts = realtime(31);
    opts.max_connections = Some(64);
    let (tcp, ws) = start_on_both_families(opts);
    let flooder = |port| SocketAddr::from(([127, 0, 0, 1], port));
    let floods = [
        flood(flooder(ws), Stall::Silent, 100),
        flood(flooder(ws), Stall::Upgraded, 100),
        flood(flooder(tcp), Stall::Silent, 100),
    ];
    // Let the flood take whatever it can before the fair client comes.
    thread::sleep(Duration::from_millis(500));
    let welcomed = bota_client_core::Endpoint::parse(&format!("ws://[::1]:{ws}"))
        .and_then(|endpoint| bota_client_core::Link::open(endpoint, FAIR_GUARD))
        .map(|link| link.with_redial(bota_client_core::Redial::OFF))
        .and_then(|mut link| bota_client_core::greet(&mut link, Role::Bot, "fair", None));
    for stop in floods {
        stop.store(true, Ordering::Relaxed);
    }
    let lobby = welcomed.expect("the fair client is welcomed");
    assert!(lobby.me.is_some(), "the welcome names the client");
}

/// The first ping of a connection the server took, read off it.
fn taken(addr: SocketAddr) -> TestClient {
    let mut client = TestClient::connect(addr);
    let mut buf = [0u8; 4096];
    let n = client.stream.read(&mut buf).expect("the server pings");
    assert!(n > 0, "the server did not take the connection");
    client.reader.push(&buf[..n]);
    client
}

/// Whether the server has let `client` go: its reading side ends.
fn let_go(client: &mut TestClient) -> bool {
    let mut buf = [0u8; 4096];
    loop {
        match client.stream.read(&mut buf) {
            Ok(0) | Err(_) => return true,
            Ok(_) => {}
        }
    }
}

/// A newcomer past the strangers' share takes a stranger's place, and at `max_connections` one
/// that said Hello keeps its place whoever comes.
#[test]
fn a_newcomer_takes_a_strangers_place_never_a_greeted_ones() {
    let mut opts = realtime(32);
    // A quarter of four: one stranger at a time.
    opts.max_connections = Some(4);
    opts.spectators = Spectators::Open;
    let (tcp, _) = start_on_both_families(opts);
    let near = SocketAddr::from(([127, 0, 0, 1], tcp));
    let far: SocketAddr = format!("[::1]:{tcp}").parse().expect("an address");
    let mut stranger = taken(near);
    let (mut fair, slot) = join_as_bot(far, "fair");
    assert_eq!(slot, Some(SlotId(0)), "the fair client is seated");
    assert!(let_go(&mut stranger), "the stranger made room");
    // Full of connections that said Hello: a newcomer finds no room, and nobody is let go.
    let _watchers: Vec<TestClient> = (0..3)
        .map(|_| {
            let mut watcher = TestClient::connect(far);
            watcher.hello(Role::Spectator, None);
            watcher.recv_until(|msg| matches!(msg, ServerMsg::Welcome { .. }).then_some(()));
            watcher
        })
        .collect();
    let mut late = TestClient::connect(near);
    assert!(
        let_go(&mut late),
        "a newcomer took a greeted connection's place"
    );
    fair.send(&ClientMsg::SetReady(true));
    fair.recv_until(|msg| match msg {
        ServerMsg::LobbyState { slots, .. } => slots
            .iter()
            .any(|seat| seat.slot == SlotId(0) && seat.ready)
            .then_some(()),
        _ => None,
    });
}

/// Players behind one address, as at a LAN party, all join at once.
#[test]
fn ten_players_from_one_address_all_join_at_once() {
    let mut opts = realtime(33);
    opts.max_connections = Some(64);
    opts.players = 10;
    let (tcp, _) = start_on_both_families(opts);
    let near = SocketAddr::from(([127, 0, 0, 1], tcp));
    let clients: Vec<TestClient> = (0..10).map(|_| TestClient::connect(near)).collect();
    let (_seated, mut slots): (Vec<TestClient>, Vec<Option<SlotId>>) = clients
        .into_iter()
        .enumerate()
        .map(|(n, client)| hello_as_bot(client, &format!("lan{n}")))
        .unzip();
    slots.sort();
    slots.dedup();
    assert_eq!(slots.len(), 10, "every player has a seat of its own");
    assert!(slots.iter().all(Option::is_some));
}

/// Threads that each dial `addr`, say Hello as a client of another wire, read the refusal and
/// then talk on, so that the server reads them on until their linger runs out, and dial again,
/// until the returned flag is raised.
fn refused_flood(addr: SocketAddr, threads: usize) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let hello = encode_frame_to_vec(&ClientMsg::hello(
        Role::Bot,
        "old build",
        None,
        !bota_core::WIRE_FINGERPRINT,
    ))
    .expect("encode");
    let pong = encode_frame_to_vec(&ClientMsg::Pong).expect("encode");
    for _ in 0..threads {
        let (stop, hello, pong) = (Arc::clone(&stop), hello.clone(), pong.clone());
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
                else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                if stream.write_all(&hello).is_err() {
                    continue;
                }
                // Whatever the server says, the refusal and its hang-up among it, the peer talks
                // on until its words meet a closed connection.
                let mut buf = [0u8; 4096];
                while !stop.load(Ordering::Relaxed) && stream.write_all(&pong).is_ok() {
                    if let Ok(0) = stream.read(&mut buf) {
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        });
    }
    stop
}

/// A peer refused at its Hello is read on for a while, and counts meanwhile as a stranger of its
/// source: one source redialling with Hellos the server refuses cannot hold every place, and a
/// fair client from elsewhere still joins.
#[test]
fn a_source_redialling_with_refused_hellos_cannot_keep_a_fair_client_out() {
    let mut opts = realtime(34);
    opts.max_connections = Some(64);
    let (tcp, _) = start_on_both_families(opts);
    let stop = refused_flood(SocketAddr::from(([127, 0, 0, 1], tcp)), 200);
    // Let the refused peers pile up before the fair client comes.
    thread::sleep(Duration::from_secs(2));
    let far: SocketAddr = format!("[::1]:{tcp}").parse().expect("an address");
    let mut fair = TestClient::connect(far);
    fair.stream
        .set_read_timeout(Some(FAIR_GUARD))
        .expect("set timeout");
    fair.send(&ClientMsg::hello(
        Role::Bot,
        "fair",
        None,
        bota_core::WIRE_FINGERPRINT,
    ));
    let mut buf = [0u8; 65536];
    let welcomed = loop {
        if let Ok(Some(msg)) = fair.reader.next_message() {
            if let ServerMsg::Welcome { slot, .. } = msg {
                break slot.is_some();
            }
            continue;
        }
        match fair.stream.read(&mut buf) {
            Ok(n @ 1..) => fair.reader.push(&buf[..n]),
            _ => break false,
        }
    };
    stop.store(true, Ordering::Relaxed);
    assert!(welcomed, "the fair client was not seated");
}

/// Players behind one address that all break off together, and are read on a while after, are
/// players still and no strangers of their address: they all join again at once.
#[test]
fn players_from_one_address_that_break_off_together_all_join_again() {
    let mut opts = realtime(35);
    opts.max_connections = Some(64);
    opts.players = 10;
    let (tcp, _) = start_on_both_families(opts);
    let near = SocketAddr::from(([127, 0, 0, 1], tcp));
    let join_all = |names: &str| -> Vec<(TestClient, Option<SlotId>)> {
        let clients: Vec<TestClient> = (0..10).map(|_| TestClient::connect(near)).collect();
        clients
            .into_iter()
            .enumerate()
            .map(|(n, client)| hello_as_bot(client, &format!("{names}{n}")))
            .collect()
    };
    let first = join_all("lan");
    assert!(first.iter().all(|(_, slot)| slot.is_some()));
    // Each breaks the protocol and stays on the line, read on by the server for a while.
    let undecodable = [1, 0, 0, 0, 0xff];
    let broken: Vec<TestClient> = first
        .into_iter()
        .map(|(mut client, _)| {
            client.stream.write_all(&undecodable).expect("send");
            client
        })
        .collect();
    for mut client in broken
        .iter()
        .map(|c| c.stream.try_clone().expect("a clone"))
    {
        let mut buf = [0u8; 4096];
        while let Ok(1..) = client.read(&mut buf) {}
    }
    let again = join_all("back");
    assert!(
        again.iter().all(|(_, slot)| slot.is_some()),
        "a player of the address was kept out"
    );
    drop(broken);
}
