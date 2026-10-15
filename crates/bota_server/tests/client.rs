//! A client library over a real socket plays a lockstep match against the
//! server: it takes a seat, folds the stream and acknowledges each tick.

use std::net::TcpListener;
use std::thread;

use bota_client_core::{
    AckPolicy, ClientSession, Ingested, Link, Perspective, SessionOptions, Transport,
};
use bota_proto::{ClientMsg, HeroId, MapId, ServerMsg, TickMode};
use bota_server::game_loop::{ServerOpts, run};

const TICKS: u32 = 90;

fn opts() -> ServerOpts {
    ServerOpts {
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 2,
        replay: None,
        seed: 3,
        map: MapId(2),
        cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        dummy_seat: true,
        ack_timeout_ticks: 150,
        order_delays: Vec::new(),
    }
}

#[test]
fn a_linked_session_folds_a_lockstep_match_it_acknowledges() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || run(listener, opts()));
    let (mut link, seated) =
        Link::join(&address, "test", Some(HeroId::SHADOW_FIEND)).expect("a seat");
    let mut session = None;
    let mut last_tick = 0;
    while last_tick < TICKS {
        let message = link.recv().expect("readable").expect("the match runs on");
        if let ServerMsg::MatchStart { info } = &message {
            let options = SessionOptions {
                perspective: Perspective::Seat(seated.slot),
                hero: Some(HeroId::SHADOW_FIEND),
                ack: AckPolicy::AfterTurn,
                forget_after_ticks: 480,
            };
            session = Some(ClientSession::start(&seated, info, options).expect("terms agree"));
            continue;
        }
        let Some(session) = session.as_mut() else {
            continue; // the lobby is still being told
        };
        if let Ingested::TickComplete { tick } = session.ingest(message, &mut ()).expect("folds") {
            last_tick = tick;
            let ack = session.end_turn().expect("a snapshot awaits its ack");
            assert_eq!(ack, tick);
            link.send(&ClientMsg::Ack { tick: ack }).expect("sent");
        }
    }
    let mut session = session.expect("the match started");
    assert_eq!(session.current().map(|view| view.tick), Some(last_tick));
    assert_eq!(
        session.end_turn(),
        None,
        "an acknowledgement is handed out once"
    );
    assert!(session.own_hero().is_some());
    drop(link);
    server
        .join()
        .expect("the server thread")
        .expect("a clean end");
}
