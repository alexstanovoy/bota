//! The client library's bot loop plays a lockstep match against the server
//! over a real socket, to its end: the loop of the `attack_move` example.

use std::net::TcpListener;
use std::thread;

use bota_client_core::{IssuedOrder, Link};
use bota_core::fountain_pos;
use bota_proto::{HeroId, MapId, Order, SlotId, Target, Team, TickMode};
use bota_server::game_loop::{DEFAULT_IDLE_TIMEOUT, ServerOpts, run};

fn opts() -> ServerOpts {
    ServerOpts {
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
    }
}

#[test]
fn the_attack_move_bot_plays_a_lockstep_match_to_its_end() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || run(listener, opts()));
    let outcome = Link::play(&address, "attack-move", HeroId::SHADOW_FIEND, |session| {
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
