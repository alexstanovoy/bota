//! One player alone: the other side has no seat, so no hero, and the match runs on as the map's
//! own rules have it. A lone player on Map2 wins by felling the enemy's first tower, and on the
//! full map by felling its Ancient; the empty side never counts as defeated, nor its seat as
//! one that never came. Cheats make it quick (a level 30 hero that cannot die, in a match
//! with the clock's start skipped).

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use bota_client_core::{IssuedOrder, Link};
use bota_proto::{Cheat, HeroId, MapId, Order, Target, Team, TickMode};
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
        players: 1,
        replay: None,
        replay_max_bytes: None,
        seed: Some(5),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map,
        cheats: true,
        immortal_structures: false,
        immortal_heroes: true,
        objectives: None,
        runes: None,
        dummy_seat: false,
        seated_start: false,
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
        empty_after: Some(Duration::from_secs(5)),
    }
}

/// A lone player's match to its end: levels, the game started, then an attack-move on the enemy
/// fountain, until the map ends it.
fn alone_on(map: MapId) -> (bota_client_core::Outcome, u32) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || run(listener, opts(map)));
    let target = bota_core::fountain_pos(bota_core::map_of(map), Team::Dire);
    let mut decisions = 0u32;
    let outcome = Link::play(&address, "alone", HeroId::SHADOW_FIEND, |session| {
        decisions += 1;
        let hero = |order| IssuedOrder::hero(order);
        match decisions {
            1 => vec![hero(Order::Cheat {
                cheat: Cheat::Levels { count: 29 },
            })],
            2 => vec![hero(Order::Cheat {
                cheat: Cheat::StartGame,
            })],
            _ if decisions.is_multiple_of(30) && session.current().is_some() => {
                vec![hero(Order::Attack {
                    target: Target::Pos(target),
                })]
            }
            _ => Vec::new(),
        }
    })
    .expect("the match is played to its end");
    let _ = server.join();
    (outcome, decisions)
}

#[test]
fn a_player_alone_on_map2_wins_by_the_first_tower() {
    let (outcome, _) = alone_on(MapId(2));
    assert_eq!(outcome.team, Team::Radiant);
    // Not at once, as an empty side lost would have it: it takes a hero's walk to a tower.
    assert!(outcome.ticks > 600, "{outcome:?}");
    assert_eq!(outcome.winner, Team::Radiant, "the tower fell: {outcome:?}");
}

#[test]
fn a_player_alone_on_the_full_map_wins_by_the_ancient() {
    let (outcome, _) = alone_on(MapId(0));
    assert_eq!(
        outcome.winner,
        Team::Radiant,
        "the Ancient fell: {outcome:?}"
    );
}
