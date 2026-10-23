//! The client library's bot loop plays a lockstep match against the server
//! over a real socket, to its end: the loop of the `attack_move` example,
//! over a compressed link as a bot far from its server would ask for.

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use bota_client_core::{IssuedOrder, Link, Transport, play};
use bota_core::fountain_pos;
use bota_proto::{ClientMsg, Codec, HeroId, MapId, Order, SlotId, Target, Team, TickMode};
use bota_server::game_loop::{DEFAULT_IDLE_TIMEOUT, DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
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
        dummy_seat: true,
        ack_timeout_ticks: 150,
        idle_timeout: Some(DEFAULT_IDLE_TIMEOUT),
        order_delays: Vec::new(),
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: None,
    }
}

#[test]
fn the_attack_move_bot_plays_a_lockstep_match_to_its_end() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || run(listener, opts()));
    let (mut link, seated) =
        Link::join(&address, "attack-move", HeroId::SHADOW_FIEND, None).expect("a seat");
    link.send(&ClientMsg::Compress(Codec::Deflate))
        .expect("the request goes out");
    let outcome = play(&mut link, seated, &mut (), |session| {
        let enemy = fountain_pos(session.map(), session.enemy());
        Some(IssuedOrder::hero(Order::Attack {
            target: Target::Pos(enemy),
        }))
    })
    .expect("the match is played to its end");
    assert_eq!((outcome.slot, outcome.team), (SlotId(0), Team::Radiant));
    assert!(outcome.ticks > 0);
    assert_eq!(outcome.rejections, 0, "only a live hero is sent orders");
    assert!(
        outcome.unsent_while_dead > 0,
        "the hero fell under the tower and was held back while dead"
    );
    server
        .join()
        .expect("the server thread")
        .expect("a clean end");
}
