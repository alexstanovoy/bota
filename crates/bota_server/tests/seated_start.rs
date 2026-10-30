//! A lobby of ten seats whose match starts with the players seated: once every seat held is
//! ready, the seats nobody holds are dropped, and no hero, bot or dummy takes their place. One
//! player is one hero; three on Map2 or the full map play as the lobby had them, a side of two
//! against a side of one.

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use bota_client_core::{Link, Lobby, Transport, greet};
use bota_proto::{ClientMsg, HeroId, MapId, Role, ServerMsg, TickMode};
use bota_server::game::OrderDelay;
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

fn opts(map: MapId) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 10,
        replay: None,
        replay_max_bytes: None,
        seed: Some(5),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map,
        cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: false,
        seated_start: true,
        ack_timeout_ticks: 150,
        // A client waits on another or on a slow line, and the machine is busy: nobody is let go
        // for silence unless the test is about that.
        idle_timeout: None,
        // One delay for each of the ten seats, as the command line gives them.
        order_delays: vec![OrderDelay { base: 1, jitter: 0 }; 10],
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: Some(Duration::from_secs(5)),
    }
}

fn serve(
    map: MapId,
) -> (
    String,
    thread::JoinHandle<std::io::Result<bota_server::game_loop::MatchEnd>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    (address, thread::spawn(move || run(listener, opts(map))))
}

/// A connection that has a seat and a pick but is not ready yet.
fn sit(address: &str, name: &str) -> (Link, Lobby) {
    let mut link = Link::connect(address, Duration::from_secs(60)).expect("connected");
    let lobby = greet(&mut link, Role::Bot, name, None).expect("a seat");
    link.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    })
    .expect("a pick");
    (link, lobby)
}

/// What the connection is told until the match is running: the picks of the match, and the
/// heroes in the world of its first snapshot.
fn until_running(link: &mut Link) -> (usize, usize) {
    let mut picks = 0;
    loop {
        match link.recv().expect("the link") {
            Some(ServerMsg::MatchStart { info }) => picks = info.picks.len(),
            Some(ServerMsg::Snapshot { view }) => return (picks, view.players.len()),
            Some(_) => {}
            None => panic!("closed before the match ran"),
        }
    }
}

/// The lobby holds ten seats until its players are ready; the match has exactly the players
/// seated: one alone is one hero, and a late third finds the seats the others closed.
fn lobby_of_ten_starts_with_the_seated(map: MapId, players: usize) {
    let (address, server) = serve(map);
    let mut seated: Vec<(Link, Lobby)> = (0..players)
        .map(|i| sit(&address, &format!("p{i}")))
        .collect();
    // Nobody is ready, so nothing has started: the last lobby state still has all ten seats.
    for (link, _) in &mut seated {
        let tell = loop {
            if let Some(ServerMsg::LobbyState { slots, .. }) = link.recv().expect("the link")
                && slots.iter().filter(|s| !s.name.is_empty()).count() == players
            {
                break slots.len();
            }
        };
        assert_eq!(tell, 10, "the lobby has ten seats");
    }
    for (link, _) in &mut seated {
        link.send(&ClientMsg::SetReady(true)).expect("ready");
    }
    for (link, _) in &mut seated {
        let (picks, heroes) = until_running(link);
        assert_eq!(picks, players, "a seat for each player seated and no other");
        assert_eq!(
            heroes, players,
            "a hero for each player seated and no other"
        );
    }
    drop(seated);
    let _ = server.join();
}

#[test]
fn one_player_in_a_lobby_of_ten_is_one_hero() {
    for map in [MapId(0), MapId(2)] {
        lobby_of_ten_starts_with_the_seated(map, 1);
    }
}

#[test]
fn three_players_play_a_side_of_two_against_a_side_of_one_on_either_map() {
    for map in [MapId(0), MapId(2)] {
        lobby_of_ten_starts_with_the_seated(map, 3);
    }
}
