//! A whole server, lobby then match, fed what hostile connections send over
//! real loopback sockets: Hellos of every role (a referee key the server
//! knows among them), picks, readiness, orders, acknowledgements, views and
//! raw bytes that are no message.
//!
//! An input is pieces: the first byte picks the connection (low two bits)
//! and whether the rest goes out as raw bytes (bit 2) rather than as one
//! framed message. Then every connection hangs up and the server must end
//! the match by itself, soon: a panic or a server that hangs fails the run.
//! Seat claims are taken out of Hellos, since a claimed seat keeps an empty
//! match going to its end.
#![no_main]

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::num::NonZeroU32;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{ClientMsg, MapId, SeatKey, TickMode, decode_payload, encode_frame_to_vec};
use bota_server::game_loop::{ServerOpts, run};
use bota_server::spectators::Spectators;
use libfuzzer_sys::fuzz_target;

/// The referee key the server is started with; the fuzzer finds it in the
/// seeds.
const REFEREE: [u8; 32] = [7; 32];
/// Pieces one input may send.
const MAX_PIECES: usize = 48;
/// How long the server may take to end once everyone hung up.
const END_WITHIN: Duration = Duration::from_secs(20);

fn opts() -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_millis(40),
        pick_penalty_time: Duration::from_millis(20),
        tactical_time: Duration::ZERO,
        same_heroes: false,
        mode: TickMode::Realtime,
        tick_rate: 1000,
        players: 2,
        replay: None,
        seed: Some(1),
        match_id: None,
        reserved: Vec::new(),
        referee_key: Some(SeatKey(REFEREE)),
        start_after: Some(Duration::from_millis(30)),
        max_connections: Some(8),
        map: MapId(2),
        cheats: true,
        immortal_structures: false,
        immortal_heroes: false,
        dummy_seat: false,
        ack_timeout_ticks: 2,
        idle_timeout: Some(Duration::from_secs(5)),
        order_delays: Vec::new(),
        messages_per_sec: NonZeroU32::new(1000).expect("nonzero"),
        max_spectators_per_seat: 2,
        spectators: Spectators::Open,
        heroes: None,
        abandon_unjoined: true,
        empty_after: Some(Duration::from_millis(50)),
    }
}

/// A connection whose incoming bytes a thread of its own reads and drops, so
/// the server never waits on it.
fn dial(address: &str) -> Option<TcpStream> {
    let stream = TcpStream::connect(address).ok()?;
    let mut reader = stream.try_clone().ok()?;
    thread::spawn(move || {
        let mut sink = [0u8; 16 * 1024];
        while matches!(reader.read(&mut sink), Ok(read) if read > 0) {}
    });
    Some(stream)
}

/// The bytes one piece puts on the wire.
fn wire(raw: bool, body: &[u8]) -> Option<Vec<u8>> {
    if raw {
        return Some(body.to_vec());
    }
    let mut msg = decode_payload::<ClientMsg>(body).ok()?;
    if let ClientMsg::Hello { key, .. } = &mut msg
        && key.as_ref().is_some_and(|key| key.0 != REFEREE)
    {
        *key = None;
    }
    encode_frame_to_vec(&msg).ok()
}

/// libFuzzer's `main` is not Rust's, which ignores SIGPIPE: without a
/// handler a write to a connection the other side closed ends the run.
fn handle_sigpipe() {
    static HANDLED: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    HANDLED.get_or_init(|| {
        let flag = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGPIPE, Arc::clone(&flag))
            .expect("a SIGPIPE handler");
        flag
    });
}

fuzz_target!(|data: &[u8]| {
    handle_sigpipe();
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || run(listener, opts()));
    let mut conns: Vec<Option<TcpStream>> = (0..4).map(|_| None).collect();
    for piece in bota_fuzz::frames(data).take(MAX_PIECES) {
        let Some((&head, body)) = piece.split_first() else {
            continue;
        };
        let at = usize::from(head & 3);
        let Some(bytes) = wire(head & 4 != 0, body) else {
            continue;
        };
        if conns[at].is_none() {
            conns[at] = dial(&address);
        }
        if let Some(conn) = conns[at].as_mut()
            && conn.write_all(&bytes).is_err()
        {
            conns[at] = None;
        }
    }
    for conn in conns.into_iter().flatten() {
        let _ = conn.shutdown(Shutdown::Both);
    }
    let hung_up = Instant::now();
    while !server.is_finished() {
        assert!(hung_up.elapsed() < END_WITHIN, "the server never ended");
        thread::sleep(Duration::from_millis(1));
    }
    server
        .join()
        .expect("the server thread")
        .expect("a clean end");
});
