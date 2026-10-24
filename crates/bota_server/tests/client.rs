//! The client library's bot loop plays a lockstep match against the server
//! over a real socket, to its end: the loop of the `attack_move` example,
//! over a compressed link as a bot far from its server would ask for.

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use bota_client_core::{IssuedOrder, Link, Transport, play};
use bota_core::fountain_pos;
use bota_proto::{ClientMsg, Codec, HeroId, ItemId, MapId, Order, SlotId, Target, Team, TickMode};
use bota_server::game_loop::{
    DEFAULT_IDLE_TIMEOUT, DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run,
};
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
        objectives: None,
        runes: None,
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
    // While the hero lies dead it still shops, as a Dota player does: one
    // Tango a death, into the stash.
    let (mut shopped, mut stashed) = (false, false);
    let outcome = play(&mut link, seated, &mut (), |session| {
        let enemy = fountain_pos(session.map(), session.enemy());
        let stash = session.own_player().and_then(|me| me.stash.as_ref());
        stashed |=
            stash.is_some_and(|stash| stash.iter().flatten().any(|item| item.id == ItemId::TANGO));
        let dead = session.own_hero().is_none();
        let buy = dead && !shopped;
        shopped = dead;
        [
            buy.then_some(IssuedOrder::hero(Order::Buy {
                item: ItemId::TANGO,
            })),
            Some(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(enemy),
            })),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
    })
    .expect("the match is played to its end");
    assert!(stashed, "a Tango bought while dead waits in the stash");
    assert_eq!((outcome.slot, outcome.team), (SlotId(0), Team::Radiant));
    assert!(outcome.ticks > 0);
    assert_eq!(
        outcome.rejections, 0,
        "only a live hero is sent a body's orders; the shop takes a dead one's"
    );
    assert!(
        outcome.unsent_while_dead > 0,
        "the hero fell under the tower and was held back while dead"
    );
    server
        .join()
        .expect("the server thread")
        .expect("a clean end");
}

/// Dota's LEAVE GAME: the seat that leaves a running match loses it at once, and the server names
/// the leaver. A seat that merely disconnects is waited for.
#[test]
fn a_seat_that_leaves_loses_the_match() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let mut options = opts();
    options.mode = TickMode::Realtime;
    options.dummy_seat = false;
    let server = thread::spawn(move || run(listener, options));
    let (mut stayer, seated) =
        Link::join(&address, "stays", HeroId::SHADOW_FIEND, None).expect("a seat");
    let (mut leaver, _) =
        Link::join(&address, "leaves", HeroId::SHADOW_FIEND, None).expect("a seat");
    let stays = thread::spawn(move || play(&mut stayer, seated, &mut (), |_| Vec::new()));
    thread::sleep(Duration::from_secs(4));
    leaver.send(&ClientMsg::LeaveGame).expect("sent");
    drop(leaver);
    let end = server.join().expect("the server thread").expect("an end");
    let MatchEnd::Over { winner, left, .. } = end else {
        panic!("the match ends with a result: {end:?}");
    };
    assert_eq!((winner, left), (Team::Radiant, Some(1)));
    stays.join().expect("the stayer").expect("it saw the end");
}
