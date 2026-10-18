//! A server with reserved seats seats only key holders, lets them come back,
//! locks their spectators to their view and tells a referee how it ended.

use std::io::Read;
use std::time::Duration;

use bota_proto::{
    ClientMsg, HeroId, MapId, PlayerId, RejectReason, Role, SeatKey, ServerMsg, SlotId, Team,
    TickMode,
};

use super::*;
use crate::lobby::ReservedSeat;

const KEYS: [SeatKey; 2] = [SeatKey([1; 32]), SeatKey([2; 32])];
const REFEREE: SeatKey = SeatKey([9; 32]);

fn reserved(seed: u64) -> ServerOpts {
    let mut opts = realtime(seed);
    opts.reserved = vec![
        ReservedSeat {
            key: KEYS[0],
            name: "zero".to_owned(),
            team: Team::Radiant,
        },
        ReservedSeat {
            key: KEYS[1],
            name: "one".to_owned(),
            team: Team::Dire,
        },
    ];
    opts.referee_key = Some(REFEREE);
    opts
}

impl TestClient {
    /// The next message, or none once the server hung up.
    pub(super) fn recv_or_hangup(&mut self) -> Option<ServerMsg> {
        let mut buf = [0u8; 65536];
        loop {
            match self.reader.next_message::<ServerMsg>().expect("decode") {
                Some(ServerMsg::Ping) => self.send(&ClientMsg::Pong),
                Some(msg) => return Some(msg),
                None => {}
            }
            match self.stream.read(&mut buf) {
                Ok(0) => return None,
                Ok(n) => self.reader.push(&buf[..n]),
                Err(error) => panic!("the server neither spoke nor hung up: {error}"),
            }
        }
    }

    pub(super) fn hello(&mut self, role: Role, key: Option<SeatKey>) {
        self.send(&ClientMsg::hello(role, "ignored", key));
    }

    /// Reads on until the server hangs up, within a bound.
    pub(super) fn assert_hung_up(&mut self) {
        for _ in 0..2000 {
            if self.recv_or_hangup().is_none() {
                return;
            }
        }
        panic!("the connection was not closed");
    }

    pub(super) fn welcome(&mut self) -> Option<SlotId> {
        self.recv_until(|msg| match msg {
            ServerMsg::Welcome { slot, .. } => Some(slot),
            _ => None,
        })
    }

    pub(super) fn match_start(&mut self) -> bota_proto::MatchInfo {
        self.recv_until(|msg| match msg {
            ServerMsg::MatchStart { info } => Some(info),
            _ => None,
        })
    }
}

/// Both seats taken by their keys, picked, ready, and the match running.
fn running_match(wire: Wire) -> (String, TestClient, TestClient) {
    let addr = serve_over(reserved(5), wire);
    let mut clients = Vec::new();
    for (key, hero) in KEYS.iter().zip([HeroId::PUDGE, HeroId::SHADOW_FIEND]) {
        let mut client = TestClient::connect(addr.as_str());
        client.hello(Role::Bot, Some(*key));
        client.welcome();
        client.send(&ClientMsg::PickHero { hero });
        client.send(&ClientMsg::SetReady(true));
        clients.push(client);
    }
    let mut second = clients.pop().expect("two clients");
    let mut first = clients.pop().expect("two clients");
    first.match_start();
    second.match_start();
    (addr, first, second)
}

impl TestClient {
    /// The refusal of a Hello, after which the server hangs up.
    pub(super) fn refusal(&mut self) -> RejectReason {
        let reason = self.recv_until(|msg| match msg {
            ServerMsg::Welcome { .. } => panic!("welcomed"),
            ServerMsg::Refused { reason } => Some(reason),
            _ => None,
        });
        self.assert_hung_up();
        reason
    }
}

over_every_wire!(
    a_key_holder_takes_its_seat_and_hero_back_only_once_the_live_connection_has_ended,
    a_seat_key_admits_only_so_many_spectators_at_once,
);

fn a_key_holder_takes_its_seat_and_hero_back_only_once_the_live_connection_has_ended(wire: Wire) {
    let (addr, old, mut other) = running_match(wire);
    let addr = addr.as_str();

    let mut early = TestClient::connect(addr);
    early.hello(Role::Bot, Some(KEYS[0]));
    assert_eq!(early.refusal(), RejectReason::SeatHeld, "the seat is held");
    let mut wrong = TestClient::connect(addr);
    wrong.hello(Role::Bot, Some(SeatKey([3; 32])));
    assert_eq!(wrong.refusal(), RejectReason::NotAdmitted);

    drop(old);
    let left = other.recv_until(|msg| match msg {
        ServerMsg::ParticipantLeft { player_id, slot } => Some((player_id, slot)),
        _ => None,
    });
    assert_eq!(
        left.1,
        Some(SlotId(0)),
        "a clean close lets the seat go at once"
    );
    assert_ne!(left.0, PlayerId(0));

    let mut back = TestClient::connect(addr);
    back.hello(Role::Bot, Some(KEYS[0]));
    assert_eq!(back.welcome(), Some(SlotId(0)));
    let info = back.match_start();
    assert_eq!(info.picks[0].hero, HeroId::PUDGE);
    let view = back.recv_until(snapshot);
    assert_eq!(view.viewer, Some(Team::Radiant));
    back.send(&move_order(1, 1000, 1000));
    let tick = back.recv_until(snapshot).tick;
    assert!(tick > view.tick);
}

#[test]
fn a_missing_wrong_or_foreign_key_gets_no_seat() {
    let addr = start_server(reserved(6));
    let attempts = [
        (Role::Bot, None),
        (Role::Player, Some(SeatKey([3; 32]))),
        (Role::Spectator, None),
        (Role::Spectator, Some(REFEREE)),
        (Role::Referee, Some(KEYS[0])),
        (Role::Referee, None),
    ];
    for (role, key) in attempts {
        let mut client = TestClient::connect(addr);
        client.hello(role, key);
        assert_eq!(
            client.refusal(),
            RejectReason::NotAdmitted,
            "{role:?} with {key:?}"
        );
    }
    // The seats are all still free for their keys.
    let mut client = TestClient::connect(addr);
    client.hello(Role::Bot, Some(KEYS[1]));
    assert_eq!(client.welcome(), Some(SlotId(1)));
}

#[test]
fn a_spectator_with_a_seat_key_sees_that_seat_only() {
    let (addr, mut first, mut second) = running_match(Wire::Tcp);
    let addr = addr.as_str();
    let mut watcher = TestClient::connect(addr);
    watcher.hello(Role::Spectator, Some(KEYS[0]));
    assert_eq!(watcher.welcome(), None);
    watcher.match_start();
    // Asking for the other seat's eyes changes nothing.
    watcher.send(&ClientMsg::ViewAs {
        seat: Some(SlotId(1)),
    });
    for seq in 1..=8 {
        first.send(&move_order(seq, 1000, 1000 + seq as i32));
        second.send(&move_order(seq, 2000, 2000 + seq as i32));
        thread::sleep(Duration::from_millis(150));
    }
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let orders = watcher.recv_until(|msg| match msg {
            ServerMsg::Orders { orders, .. } => Some(orders),
            _ => None,
        });
        seen.extend(orders);
    }
    assert!(seen.iter().all(|given| given.slot == SlotId(0)));
    let view = watcher.recv_until(snapshot);
    assert_eq!(view.viewer, Some(Team::Radiant));
}

fn a_seat_key_admits_only_so_many_spectators_at_once(wire: Wire) {
    let mut opts = reserved(9);
    opts.max_spectators_per_seat = 2;
    let addr = serve_over(opts, wire);
    let addr = addr.as_str();
    let watch = |key: SeatKey| {
        let mut client = TestClient::connect(addr);
        client.hello(Role::Spectator, Some(key));
        client
    };
    let mut first = watch(KEYS[0]);
    assert_eq!(first.welcome(), None);
    let mut second = watch(KEYS[0]);
    assert_eq!(second.welcome(), None);
    assert_eq!(watch(KEYS[0]).refusal(), RejectReason::NotAdmitted);
    assert_eq!(watch(KEYS[1]).welcome(), None, "each seat counts its own");
    let mut player = TestClient::connect(addr);
    player.hello(Role::Bot, Some(KEYS[0]));
    assert_eq!(
        player.welcome(),
        Some(SlotId(0)),
        "the seat itself is not one"
    );

    drop(first);
    // The player is told the lobby on its own arrival, then once the departure is heard.
    for _ in 0..2 {
        player.recv_until(|msg| matches!(msg, ServerMsg::LobbyState { .. }).then_some(()));
    }
    let mut third = watch(KEYS[0]);
    assert_eq!(third.welcome(), None);
}

#[test]
fn a_referee_hears_the_lobby_and_the_end_but_no_ticks_of_a_match_nobody_joined() {
    let mut opts = reserved(8);
    opts.mode = TickMode::Lockstep;
    opts.map = MapId(2);
    opts.start_after = Some(Duration::from_millis(300));
    let addr = start_server(opts);
    let mut referee = TestClient::connect(addr);
    referee
        .stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .expect("timeout");
    referee.hello(Role::Referee, Some(REFEREE));
    assert_eq!(referee.welcome(), None);

    let mut started = false;
    let (winner, stats) = loop {
        match referee
            .recv_or_hangup()
            .expect("the match ends before the server hangs up")
        {
            ServerMsg::MatchStart { info } => {
                // Nobody picked: both seats got a hero of the server's own choosing.
                assert_eq!(info.picks.len(), 2);
                started = true;
            }
            ServerMsg::MatchOver { winner, stats } => break (winner, stats),
            ServerMsg::Snapshot { .. } | ServerMsg::Events { .. } | ServerMsg::Orders { .. } => {
                panic!("a referee is told no ticks")
            }
            _ => {}
        }
    };
    assert!(started);
    // Nobody played, so the waves fight on alone: most seeds draw at the time cap, on some a
    // wave takes a tower first. Either way the match plays to its own end.
    let cap = bota_core::MAP2_TICK_CAP;
    match winner {
        Team::Neutral => assert_eq!(stats.duration, cap, "a draw comes at the time cap"),
        _ => assert!(stats.duration < cap, "a side wins before the cap"),
    }
    assert_eq!(stats.slots.len(), 2);
}
