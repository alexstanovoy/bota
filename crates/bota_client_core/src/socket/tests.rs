//! How the threaded [`Net`] ends: a clean close is no message, a broken
//! connection is an error.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{
    ClientMsg, HeroId, PlayerId, RejectReason, Role, SeatKey, ServerMsg, SlotId, TickMode,
    encode_frame_to_vec,
};

use crate::{Endpoint, Link, Net, Redial, Transport};

fn hello() -> ClientMsg {
    ClientMsg::hello(Role::Bot, "net", None, bota_core::WIRE_FINGERPRINT)
}

#[test]
fn a_clean_close_is_no_message() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let server = thread::spawn(move || {
        let (mut peer, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 256];
        let _ = peer.read(&mut buf).expect("the hello arrives");
    });
    let mut net = Net::connect_with(&addr, Redial::OFF).expect("connect");
    net.send(&hello()).expect("send");
    server.join().expect("the server side ends");
    assert!(net.recv().expect("no error").is_none());
}

#[test]
fn a_reset_connection_is_an_error() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let server = thread::spawn(move || {
        let (peer, _) = listener.accept().expect("accept");
        // Closing with the hello unread makes the kernel reset the connection: peeking waits
        // for it to be there, and leaves it unread.
        let mut buf = [0u8; 256];
        assert!(peer.peek(&mut buf).expect("the hello arrives") > 0);
        drop(peer);
    });
    let mut net = Net::connect_with(&addr, Redial::OFF).expect("connect");
    net.send(&hello()).expect("send");
    server.join().expect("the server side ends");
    net.recv().expect_err("a reset is not a clean close");
}

#[test]
fn a_redial_lasts_until_the_server_answers_on_the_fresh_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let (greeted, hello_again) = mpsc::channel();
    let (answer, go) = mpsc::channel();
    let server = thread::spawn(move || {
        let (first, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 256];
        // Dropped with the hello unread, the connection is reset: a break to dial again after.
        assert!(first.peek(&mut buf).expect("the hello arrives") > 0);
        drop(first);
        let (mut fresh, _) = listener.accept().expect("the client dials again");
        let _ = fresh.read(&mut buf).expect("the hello arrives again");
        // Connected and greeted, but nothing back until the test says so: a black hole.
        greeted.send(()).expect("the test listens");
        go.recv().expect("the test answers");
        fresh
            .write_all(&encode_frame_to_vec(&ServerMsg::Ping).expect("encode"))
            .expect("ping");
        // Held open until the client has taken the ping in.
        go.recv().expect_err("the test is done");
    });
    let mut net = Net::connect(&addr).expect("connect");
    net.send(&hello()).expect("send");
    hello_again.recv().expect("the fresh connection is greeted");
    assert!(net.is_redialing(), "no word from the fresh connection yet");
    answer.send(()).expect("the server waits");
    let deadline = Instant::now() + Duration::from_secs(300);
    while net.is_redialing() {
        assert!(Instant::now() < deadline, "the server answered");
        thread::sleep(Duration::from_millis(5));
    }
    drop(answer);
    server.join().expect("the server side ends");
}

/// A server that floods a caller who stalled: the reader stops taking bytes
/// once the inbox holds its bound of payload, so the server's writes stall
/// too. It took a thousand messages whatever their size, and a decoded
/// message can take sixteen times its payload.
#[test]
fn a_stalled_caller_holds_a_bounded_inbox_of_a_flooding_server() {
    use bota_proto::{LobbySlot, SlotId, Team};
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let slot = LobbySlot {
        slot: SlotId(0),
        team: Team::Radiant,
        name: "x".repeat(64 * 1024),
        role: None,
        hero: None,
        ready: false,
    };
    let frame = encode_frame_to_vec(&ServerMsg::LobbyState {
        slots: vec![slot],
        same_heroes: true,
        pick: bota_proto::PickPhase::Waiting,
        heroes: Vec::new(),
        catalogue: bota_core::Catalogue::FULL.prefix(),
    })
    .expect("encode");
    let server = thread::spawn(move || {
        let (mut peer, _) = listener.accept().expect("accept");
        peer.set_write_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let mut written = 0usize;
        while written < 1024 * frame.len() && peer.write_all(&frame).is_ok() {
            written += frame.len();
        }
        written
    });
    let net = Net::connect_with(&addr, Redial::OFF).expect("connect");
    let written = server.join().expect("the server side ends");
    drop(net);
    assert!(
        written < 40 * 1024 * 1024,
        "{written} bytes were taken in for a caller that read none"
    );
}

/// A client that comes back to its seat with the seat's key (a new process, a new link) may
/// find the server still holding it for the old connection, which the server has not yet seen
/// end: it is refused (SeatHeld), and dials again by its policy until the seat is let go.
#[test]
fn a_seat_still_held_for_the_old_connection_is_taken_once_it_is_let_go() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let server = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let (mut first, _) = listener.accept().expect("accept");
        let _ = first.read(&mut buf).expect("the hello arrives");
        let held = ServerMsg::Refused {
            reason: RejectReason::SeatHeld,
        };
        first
            .write_all(&encode_frame_to_vec(&held).expect("encode"))
            .expect("refused");
        drop(first);
        let (mut fresh, _) = listener.accept().expect("the client dials again");
        let _ = fresh.read(&mut buf).expect("the hello arrives again");
        let welcome = ServerMsg::Welcome {
            player_id: PlayerId(1),
            slot: Some(SlotId(0)),
            tick_rate: 30,
            mode: TickMode::Lockstep,
        };
        fresh
            .write_all(&encode_frame_to_vec(&welcome).expect("encode"))
            .expect("welcome");
        // Held open until the client has its seat.
        while fresh.read(&mut buf).is_ok_and(|read| read > 0) {}
    });
    let redial = Redial {
        attempts: 3,
        first_wait: Duration::from_millis(1),
        longest_wait: Duration::from_millis(1),
    };
    let endpoint = Endpoint::parse(&addr).expect("an endpoint");
    let key = Some(SeatKey([7; 32]));
    let (link, seated) = Link::join_at(
        endpoint,
        "net",
        HeroId::SHADOW_FIEND,
        key,
        Duration::from_secs(30),
        redial,
    )
    .expect("the seat, once the old connection let it go");
    assert_eq!(seated.slot, SlotId(0));
    drop(link);
    server.join().expect("the server side ends");
}

/// Reads the frames of one hello and the catalogue after it from a fresh connection.
fn read_greeting(peer: &mut std::net::TcpStream) -> (ClientMsg, ClientMsg) {
    let mut frames = bota_proto::FrameReader::new();
    let mut said = Vec::new();
    let mut buf = [0u8; 4096];
    while said.len() < 2 {
        let read = peer.read(&mut buf).expect("the greeting arrives");
        assert!(read > 0, "the client closed before its catalogue");
        frames.push(&buf[..read]);
        while let Some(msg) = frames.next_message::<ClientMsg>().expect("a client frame") {
            said.push(msg);
        }
    }
    (said[0].clone(), said[1].clone())
}

/// A server of an earlier wire this build admits (`bota_core::ADMITTED_WIRES`) answers this
/// build's hello with `WireMismatch` naming its own and closes: the client dials again speaking
/// that wire, says that build's items, and is welcomed: each wire it admits, none when it admits
/// none. Any other wire stays refused (`a_server_of_a_wire_not_admitted_is_not_dialed_again`).
#[test]
fn a_server_of_an_admitted_earlier_wire_is_dialed_again_in_its_wire() {
    for &earlier in bota_core::ADMITTED_WIRES {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("accept");
            let (hello, catalogue) = read_greeting(&mut first);
            let mismatch = ServerMsg::WireMismatch {
                server_fingerprint: earlier.wire,
            };
            first
                .write_all(&encode_frame_to_vec(&mismatch).expect("encode"))
                .expect("mismatch");
            drop(first);
            let (mut fresh, _) = listener.accept().expect("the client dials again");
            let again = read_greeting(&mut fresh);
            let welcome = ServerMsg::Welcome {
                player_id: PlayerId(1),
                slot: Some(SlotId(0)),
                tick_rate: 30,
                mode: TickMode::Lockstep,
            };
            fresh
                .write_all(&encode_frame_to_vec(&welcome).expect("encode"))
                .expect("welcome");
            let mut buf = [0u8; 4096];
            while fresh.read(&mut buf).is_ok_and(|read| read > 0) {}
            ((hello, catalogue), again)
        });
        let redial = Redial {
            attempts: 3,
            first_wait: Duration::from_millis(1),
            longest_wait: Duration::from_millis(1),
        };
        let endpoint = Endpoint::parse(&addr).expect("an endpoint");
        let (link, seated) = Link::join_at(
            endpoint,
            "net",
            HeroId::SHADOW_FIEND,
            None,
            Duration::from_secs(30),
            redial,
        )
        .expect("a seat on the earlier wire");
        assert_eq!(seated.slot, SlotId(0));
        drop(link);
        let ((hello, catalogue), (hello_again, catalogue_again)) =
            server.join().expect("the server side ends");
        let wire = |hello: &ClientMsg| match hello {
            ClientMsg::Hello {
                wire_fingerprint, ..
            } => *wire_fingerprint,
            other => panic!("a hello first, not {other:?}"),
        };
        assert_eq!(wire(&hello), bota_core::WIRE_FINGERPRINT);
        assert_eq!(
            catalogue,
            ClientMsg::Catalogue(bota_core::Catalogue::FULL.prefix())
        );
        assert_eq!(wire(&hello_again), earlier.wire);
        assert_eq!(catalogue_again, ClientMsg::Catalogue(earlier.catalogue));
    }
}

/// A link dials one server through `redial` and takes a seat as a lone Shadow Fiend.
fn join_once(addr: &str) -> std::io::Result<crate::Seated> {
    let redial = Redial {
        attempts: 3,
        first_wait: Duration::from_millis(1),
        longest_wait: Duration::from_millis(1),
    };
    let endpoint = Endpoint::parse(addr).expect("an endpoint");
    Link::join_at(
        endpoint,
        "net",
        HeroId::SHADOW_FIEND,
        None,
        Duration::from_secs(30),
        redial,
    )
    .map(|(_, seated)| seated)
}

/// Answers a connection's greeting with a `WireMismatch` naming `named`, and closes it; the
/// hello's wire comes back.
fn mismatch(peer: &mut std::net::TcpStream, named: u64) -> u64 {
    let (hello, _) = read_greeting(peer);
    let mismatch = ServerMsg::WireMismatch {
        server_fingerprint: named,
    };
    peer.write_all(&encode_frame_to_vec(&mismatch).expect("encode"))
        .expect("mismatch");
    match hello {
        ClientMsg::Hello {
            wire_fingerprint, ..
        } => wire_fingerprint,
        other => panic!("a hello first, not {other:?}"),
    }
}

/// Whether anyone dialed the listener since: checked once the client gave up, so nothing can
/// come later.
fn dialed_again(listener: &TcpListener) -> bool {
    listener.set_nonblocking(true).expect("nonblocking");
    match listener.accept() {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
        Err(error) => panic!("the listener failed: {error}"),
    }
}

/// A server of a wire this build does not admit stays refused: no second dial.
#[test]
fn a_server_of_a_wire_not_admitted_is_not_dialed_again() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let other = bota_core::WIRE_FINGERPRINT ^ 1;
    assert!(!bota_core::admits_wire(other));
    let (gave_up, given_up) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().expect("accept");
        mismatch(&mut first, other);
        drop(first);
        given_up.recv().expect("the client gave up");
        dialed_again(&listener)
    });
    let refused = join_once(&addr).expect_err("refused");
    gave_up.send(()).expect("the server waits");
    assert!(
        refused.to_string().contains(&format!("{other:016x}")),
        "{refused}"
    );
    assert!(
        !server.join().expect("the server side ends"),
        "dialed again"
    );
}

/// One fallback a link: a server that, spoken to in the admitted wire it named, names yet another
/// wire the client admits (here this build's own, which the client began with) is not followed
/// round; the client stops with the three wires named, and dials no third time. For each wire it
/// admits, none when it admits none.
#[test]
fn a_second_wire_named_after_the_fallback_is_a_hard_failure() {
    for &earlier in bota_core::ADMITTED_WIRES {
        let own = bota_core::WIRE_FINGERPRINT;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let (gave_up, given_up) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("accept");
            let asked = mismatch(&mut first, earlier.wire);
            drop(first);
            let (mut second, _) = listener.accept().expect("the client dials again");
            let fell_back = mismatch(&mut second, own);
            drop(second);
            given_up.recv().expect("the client gave up");
            (asked, fell_back, dialed_again(&listener))
        });
        let refused = join_once(&addr).expect_err("refused");
        gave_up.send(()).expect("the server waits");
        let (asked, fell_back, third) = server.join().expect("the server side ends");
        assert_eq!((asked, fell_back), (own, earlier.wire));
        assert!(!third, "dialed a third time");
        assert_eq!(
            refused.to_string(),
            format!(
                "the server named wire {own:016x} after this client had fallen back from {own:016x} \
to {:016x}, the one fallback a link makes; not dialing again",
                earlier.wire
            )
        );
    }
}
