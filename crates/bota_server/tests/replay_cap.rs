//! A replay under a size cap: the server closes the file whole before it grows past the cap, plays
//! the match on, and ends it with its result and the news that the replay was cut short.

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use bota_client_core::{IssuedOrder, Link, UnpackedReplay, play};
use bota_core::fountain_pos;
use bota_proto::{FrameReader, HeroId, MapId, Order, ReplayRecord, ServerMsg, Target, TickMode};
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
        seed: Some(3),
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
        dummy_seat: true,
        ack_timeout_ticks: 150,
        // A client waits on another or on a slow line, and the machine is busy: nobody is let go
        // for silence unless the test is about that.
        idle_timeout: None,
        order_delays: Vec::new(),
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: None,
        seated_start: false,
    }
}

/// A cap the opening of the match (the full snapshots) fills long before its end.
const CAP: u64 = 64 << 10;

#[test]
fn a_replay_over_its_cap_is_closed_whole_and_the_match_is_played_on() {
    let path = std::env::temp_dir().join(format!("bota-replay-cap-{}.brp", std::process::id()));
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let mut options = opts();
    options.replay = Some(path.clone());
    options.replay_max_bytes = Some(CAP);
    let server = thread::spawn(move || run(listener, options));
    let (mut link, seated) =
        Link::join(&address, "attack-move", HeroId::SHADOW_FIEND, None).expect("a seat");
    play(&mut link, seated, &mut (), |session| {
        let enemy = fountain_pos(session.map(), session.enemy());
        Some(IssuedOrder::hero(Order::Attack {
            target: Target::Pos(enemy),
        }))
    })
    .expect("the match is played to its end");
    let end = server
        .join()
        .expect("the server thread")
        .expect("a clean end");
    let MatchEnd::Over {
        replay_truncated, ..
    } = end
    else {
        panic!("the match ends with a result: {end:?}");
    };
    assert!(replay_truncated, "the result says the replay was cut short");

    let size = std::fs::metadata(&path).expect("the replay").len();
    assert!(size <= CAP, "{size} bytes under the cap of {CAP}");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut UnpackedReplay::new(std::fs::File::open(&path).expect("the replay")),
        &mut bytes,
    )
    .expect("a stream closed whole unpacks to its end");
    let _ = std::fs::remove_file(&path);
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    let (mut records, mut over) = (0, false);
    while let Some(record) = reader
        .next_message::<ReplayRecord>()
        .expect("a whole record")
    {
        records += 1;
        over |= matches!(record, ReplayRecord::Msg(ServerMsg::MatchOver { .. }));
    }
    assert!(records > 1, "the replay holds the match's opening");
    assert!(!over, "the match went on past the end of the replay");
}
