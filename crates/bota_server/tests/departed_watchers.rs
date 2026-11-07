//! A key-locked spectator loses its private team view when its seat leaves permanently.

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use bota_client_core::{Link, Transport, greet, join, play};
use bota_proto::{ClientMsg, HeroId, MapId, Role, SeatKey, ServerMsg, SlotId, Team, TickMode};
use bota_server::game::OrderDelay;
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run};
use bota_server::lobby::ReservedSeat;
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

fn seat(key: u8, name: &str, team: Team) -> ReservedSeat {
    ReservedSeat {
        key: SeatKey([key; 32]),
        name: name.to_owned(),
        team,
        kind: None,
    }
}

fn opts(reserved: Vec<ReservedSeat>) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 3,
        replay: None,
        replay_max_bytes: None,
        meta: None,
        meta_trailer: false,
        seed: Some(5),
        match_id: None,
        reserved,
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(2),
        cheats: false,
        lobby_cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: false,
        seated_start: false,
        ack_timeout_ticks: 150,
        idle_timeout: None,
        order_delays: vec![OrderDelay { base: 1, jitter: 0 }; 3],
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: Some(Duration::from_secs(5)),
    }
}

/// Acknowledges every tick; once `leave_at` is acknowledged it sends LeaveGame and hangs up.
struct LeaveAndHangUp {
    link: Link,
    leave_at: u32,
}

impl Transport for LeaveAndHangUp {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.link.recv()
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        self.link.send(msg)?;
        match msg {
            ClientMsg::Ack { tick } if *tick >= self.leave_at => {
                self.link.send(&ClientMsg::LeaveGame)?;
                Err(std::io::Error::other("hung up after leaving"))
            }
            _ => Ok(()),
        }
    }
}

#[test]
fn a_key_locked_watcher_of_a_seat_that_left_for_good_gets_no_more_of_its_team_view() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let reserved = vec![
        seat(1, "host", Team::Radiant),
        seat(2, "friend", Team::Radiant),
        seat(3, "enemy", Team::Dire),
    ];
    let server = thread::spawn(move || run(listener, opts(reserved)));
    let (watching, admitted) = std::sync::mpsc::channel();
    // Admit the watcher before any player can start or leave the match.
    let watcher = {
        let address = address.clone();
        thread::spawn(move || {
            let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
            greet(
                &mut link,
                Role::Spectator,
                "watcher",
                Some(SeatKey([1; 32])),
            )
            .expect("the key admits a watcher through the host's seat");
            watching.send(()).expect("watcher admitted");
            let mut views: Vec<(u32, Option<Team>)> = Vec::new();
            while let Some(message) = link.recv().expect("the watcher's feed") {
                match message {
                    ServerMsg::Ping => link.send(&ClientMsg::Pong).expect("pong"),
                    ServerMsg::Snapshot { view } => views.push((view.tick, view.viewer)),
                    ServerMsg::MatchOver { .. } => break,
                    _ => {}
                }
            }
            views
        })
    };
    admitted
        .recv_timeout(Duration::from_secs(300))
        .expect("watcher admission");
    let host = {
        let address = address.clone();
        thread::spawn(move || {
            let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
            let seated = join(
                &mut link,
                "host",
                HeroId::SHADOW_FIEND,
                Some(SeatKey([1; 32])),
            )
            .expect("a seat");
            let mut leaving = LeaveAndHangUp { link, leave_at: 60 };
            let _ = play(&mut leaving, seated, &mut (), |_| Vec::new());
        })
    };
    let friend = {
        let address = address.clone();
        thread::spawn(move || {
            let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
            let seated = join(
                &mut link,
                "friend",
                HeroId::SHADOW_FIEND,
                Some(SeatKey([2; 32])),
            )
            .expect("a seat");
            play(&mut link, seated, &mut (), |_| Vec::new()).expect("played");
        })
    };
    let enemy = {
        let address = address.clone();
        thread::spawn(move || {
            let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
            let seated = join(
                &mut link,
                "enemy",
                HeroId::SHADOW_FIEND,
                Some(SeatKey([3; 32])),
            )
            .expect("a seat");
            let mut leaving = LeaveAndHangUp {
                link,
                leave_at: 240,
            };
            let _ = play(&mut leaving, seated, &mut (), |_| Vec::new());
        })
    };
    host.join().expect("the host thread");
    friend.join().expect("the friend thread");
    enemy.join().expect("the enemy thread");
    let views = watcher.join().expect("the watcher thread");
    let end = server.join().expect("the server").expect("it ran");
    let MatchEnd::Over { report, .. } = end else {
        panic!("the match ends with a result: {end:?}");
    };
    let left = report
        .stats
        .leaves
        .iter()
        .find(|left| left.slot == SlotId(0))
        .map(|left| left.tick)
        .expect("the host's departure is reported");
    assert!(
        views
            .iter()
            .any(|(tick, viewer)| *tick <= left && *viewer == Some(Team::Radiant)),
        "precondition: the watcher saw its team's view before the departure at tick {left}"
    );
    let leaked: Vec<u32> = views
        .iter()
        .filter(|(tick, viewer)| *tick > left && *viewer == Some(Team::Radiant))
        .map(|(tick, _)| *tick)
        .collect();
    assert!(
        leaked.is_empty(),
        "the seat left for good at tick {left}, yet its key-locked watcher still received its team's view {} times, first at tick {}",
        leaked.len(),
        leaked.first().copied().unwrap_or(0)
    );
}
