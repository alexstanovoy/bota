//! A seat that leaves for good during the picks ([`ClientMsg::LeaveGame`] in the lobby) cancels
//! the match, as Dota cancels one abandoned during hero selection: the server ends with no
//! result, and the seat that stayed is never shown a match. The leaver here does what a client's
//! LEAVE does after a DISCONNECT: it takes its seat again with its key, then leaves.

use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use bota_client_core::{Link, Transport, greet};
use bota_proto::{ClientMsg, HeroId, MapId, Role, SeatKey, ServerMsg, TickMode};
use bota_server::game::OrderDelay;
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

fn opts() -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 2,
        replay: None,
        replay_max_bytes: None,
        meta: None,
        meta_trailer: false,
        seed: Some(5),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(2),
        cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: false,
        seated_start: false,
        ack_timeout_ticks: 150,
        // A client waits on another or on a slow line, and the machine is busy: nobody is let go
        // for silence unless the test is about that.
        idle_timeout: None,
        order_delays: vec![OrderDelay { base: 1, jitter: 0 }; 2],
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: Some(Duration::from_secs(5)),
    }
}

/// A seat taken with `key` and a hero picked, not ready.
fn sit(address: &str, name: &str, key: SeatKey) -> Link {
    let mut link = Link::connect(address, Duration::from_secs(60)).expect("connected");
    greet(&mut link, Role::Player, name, Some(key)).expect("a seat");
    link.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    })
    .expect("a pick");
    link
}

#[test]
fn a_seat_that_leaves_during_the_picks_cancels_the_match() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let (ended, end) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = ended.send(run(listener, opts()));
    });
    let (leaver, stayer) = (SeatKey([1; 32]), SeatKey([2; 32]));
    let mut stays = sit(&address, "stays", stayer);
    // The leaver's seat, left as a DISCONNECT leaves it, and taken again with its key: the
    // server may hold it a moment for the broken connection.
    drop(sit(&address, "leaves", leaver));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut back = loop {
        let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
        match greet(&mut link, Role::Player, "leaves", Some(leaver)) {
            Ok(_) => break link,
            Err(error) => assert!(
                Instant::now() < deadline,
                "the seat never came back: {error}"
            ),
        }
        thread::sleep(Duration::from_millis(100));
    };
    back.send(&ClientMsg::LeaveGame).expect("left");
    let end = end
        .recv_timeout(Duration::from_secs(30))
        .expect("the server ended")
        .expect("ran");
    assert_eq!(
        end,
        MatchEnd::Abandoned,
        "the match is cancelled: no result"
    );
    // The seat that stayed was never shown a match.
    loop {
        match stays.recv() {
            Ok(Some(ServerMsg::MatchStart { .. } | ServerMsg::Snapshot { .. })) => {
                panic!("a match began after a seat left during the picks")
            }
            Ok(Some(_)) => {}
            // The server closed it: the match is over for it too.
            Ok(None) => break,
            Err(error) => panic!("the seat that stayed was not let go: {error}"),
        }
    }
}
