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
            kind: None,
            key: KEYS[0],
            name: "zero".to_owned(),
            team: Team::Radiant,
        },
        ReservedSeat {
            kind: None,
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
            match self.reader.next_message().expect("decode") {
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
        self.send(&ClientMsg::hello(
            role,
            "ignored",
            key,
            bota_core::WIRE_FINGERPRINT,
        ));
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
    a_seat_that_left_for_good_cannot_rejoin_a_running_match,
    a_seat_that_left_for_good_cannot_order_or_speak_on_its_open_connection,
    a_disconnected_seat_can_rejoin_during_picks_or_a_running_match,
    a_seat_key_admits_only_so_many_spectators_at_once,
);

/// Three seats, so one Radiant seat leaving does not forfeit its team's match.
fn three_seats(wire: Wire, reserved_seats: bool, started: bool) -> (String, Vec<TestClient>) {
    let addr = serve_over(three_seat_opts(reserved_seats), wire);
    let clients = join_three_seats(&addr, started);
    (addr, clients)
}

fn three_seat_opts(reserved_seats: bool) -> ServerOpts {
    let mut opts = reserved(55);
    opts.players = 3;
    opts.order_delays = vec![OrderDelay::NONE; 3];
    opts.reserved.push(ReservedSeat {
        kind: None,
        key: SeatKey([3; 32]),
        name: "two".to_owned(),
        team: Team::Radiant,
    });
    if !reserved_seats {
        opts.reserved.clear();
    }
    // No phase ends while the test drives another socket on a busy machine.
    opts.pick_time = HANG_GUARD * 2;
    opts.mode = TickMode::Lockstep;
    opts.ack_timeout_ticks = 30 * HANG_GUARD.as_secs() as u32;
    opts
}

fn join_three_seats(addr: &str, started: bool) -> Vec<TestClient> {
    let mut clients = Vec::new();
    for key in [KEYS[0], KEYS[1], SeatKey([3; 32])] {
        let mut client = TestClient::connect(addr);
        client.hello(Role::Bot, Some(key));
        client.welcome();
        client.send(&ClientMsg::PickHero {
            hero: HeroId::SHADOW_FIEND,
        });
        client.send(&ClientMsg::SetReady(started));
        clients.push(client);
    }
    if started {
        for client in &mut clients {
            client.match_start();
        }
    }
    clients
}

fn a_seat_that_left_for_good_cannot_rejoin_a_running_match(wire: Wire) {
    for reserved_seats in [true, false] {
        let (addr, mut clients) = three_seats(wire, reserved_seats, true);
        let mut leaver = clients.remove(0);
        leaver.send(&ClientMsg::LeaveGame);
        // The close is a barrier: LeaveGame was heard and the old connection released.
        leaver.leave();
        let mut back = TestClient::connect(addr.as_str());
        back.hello(Role::Bot, Some(KEYS[0]));
        assert_eq!(back.refusal(), RejectReason::NotAdmitted);
        // The client library exposes the final admission refusal to its caller as well.
        let mut link = bota_client_core::Net::connect(&addr).expect("connected");
        let error = bota_client_core::greet(&mut link, Role::Bot, "back", Some(KEYS[0]))
            .expect_err("a seat left for good is refused");
        assert!(
            matches!(
                error
                    .get_ref()
                    .and_then(|error| error.downcast_ref::<bota_client_core::SessionError>()),
                Some(bota_client_core::SessionError::Refused(
                    RejectReason::NotAdmitted
                ))
            ),
            "{error}"
        );
        // The teammate still plays: refusal did not cancel the match or disturb its seat.
        clients[1].say_all_and_hear("still playing");
    }
}

fn leave_then_order_and_speak(client: &mut TestClient, tick: u32) {
    let mut frames = Vec::new();
    for msg in [
        ClientMsg::LeaveGame,
        ClientMsg::Order {
            seq: 77,
            unit: None,
            order: Order::Buy {
                item: bota_proto::ItemId::BOOTS,
            },
            seen: Some(tick),
            queue: false,
        },
        say(Audience::All, "after leaving"),
        ClientMsg::Ack { tick },
    ] {
        frames.extend(encode_frame_to_vec(&msg).expect("message frame"));
    }
    client
        .stream
        .write_all(&frames)
        .expect("leave then more messages");
}

fn a_seat_that_left_for_good_cannot_order_or_speak_on_its_open_connection(wire: Wire) {
    let (_addr, mut clients) = three_seats(wire, true, true);
    let first = clients[0].recv_until(snapshot);
    clients[1].recv_until(snapshot);
    let teammate = clients[2].recv_until(snapshot);
    let hero = teammate
        .player(SlotId(0))
        .expect("allied seat")
        .unit
        .expect("allied hero");
    let items = teammate.unit(hero).expect("visible ally").items.clone();
    // Before the fix, the last Ack can advance this tick only after the order and chat were
    // heard. After the fix, departed seats owe no Ack, and all their messages are inert.
    leave_then_order_and_speak(&mut clients[0], first.tick);
    for client in &mut clients[1..] {
        client.send(&ClientMsg::Ack { tick: first.tick });
    }
    let mut told = clients[2].collect_through_tick(first.tick + 1);
    let after = told
        .iter()
        .find_map(|msg| snapshot(msg.clone()))
        .expect("next tick");
    assert_eq!(
        after.unit(hero).expect("visible ally").items,
        items,
        "an abandoned seat cannot buy"
    );
    // No further Ack comes from the abandoned socket; its team can still advance again.
    for client in &mut clients[1..] {
        client.send(&ClientMsg::Ack {
            tick: first.tick + 1,
        });
    }
    told.extend(clients[2].collect_through_tick(first.tick + 2));
    // The old socket stayed open while the surviving seats advanced the match. Closing it
    // now is a barrier for every preceding message, including any chat queued after the tick.
    clients.remove(0).leave();
    told.extend(clients[1].collect_until(|msg| {
        matches!(
            msg,
            ServerMsg::ParticipantLeft {
                slot: Some(SlotId(0)),
                ..
            }
        )
    }));
    assert!(
        !told.iter().any(|msg| matches!(
            msg,
            ServerMsg::Heard(heard)
                if heard.line.body == Body::Text("after leaving".to_owned())
        )),
        "an abandoned seat cannot speak"
    );
}

#[test]
fn an_abandoned_socket_cannot_keep_an_empty_match_alive() {
    let mut opts = three_seat_opts(true);
    opts.empty_after = Some(Duration::ZERO);
    let server = start_ending_server(opts);
    let mut clients = join_three_seats(&server.addr.to_string(), true);
    let first = clients[0].recv_until(snapshot).tick;
    for client in &mut clients[1..] {
        client.recv_until(snapshot);
        client.send(&ClientMsg::Ack { tick: first });
    }
    clients[0].send(&ClientMsg::LeaveGame);
    let next = clients[0].recv_until(snapshot).tick;
    assert_eq!(next, first + 1, "the departed seat owes no Ack");
    let mut abandoned = clients.remove(0);
    // The other sockets close without acknowledging `next`. Only the abandoned socket remains.
    for client in clients {
        client.leave();
    }
    while let Some(msg) = abandoned.recv_or_hangup() {
        match msg {
            ServerMsg::Snapshot { view } => {
                panic!("the empty match advanced to tick {}", view.tick)
            }
            ServerMsg::MatchOver { .. } => {
                panic!("the empty match completed instead of abandoning")
            }
            _ => {}
        }
    }
    assert_eq!(server.end(), MatchEnd::Abandoned);
}

#[test]
fn a_keyless_seat_that_leaves_still_forfeits_against_a_dummy() {
    let mut opts = realtime(57);
    opts.dummy_seat = true;
    opts.mode = TickMode::Lockstep;
    let server = start_ending_server(opts);
    let (mut leaver, slot) = join_as_bot(server.addr, "leaver");
    leaver.match_start();
    leaver.recv_until(snapshot);
    leaver.send(&ClientMsg::LeaveGame);
    let end = server.end();
    assert!(
        matches!(end, MatchEnd::Over { winner: Team::Dire, left: Some(left), .. } if Some(SlotId(left)) == slot),
        "leaving forfeits instead of cancelling a running match: {end:?}"
    );
}

#[test]
fn a_leave_during_picks_cancels_even_with_another_hello_queued() {
    let mut opts = reserved(56);
    opts.pick_time = HANG_GUARD * 2;
    let server = start_ending_server(opts);
    let mut stayer = TestClient::connect(server.addr);
    stayer.hello(Role::Bot, Some(KEYS[1]));
    stayer.welcome();
    let mut leaver = TestClient::connect(server.addr);
    leaver.hello(Role::Bot, Some(KEYS[0]));
    leaver.welcome();
    // One write preserves the wire order even when the reader batches both messages.
    let mut frames = encode_frame_to_vec(&ClientMsg::LeaveGame).expect("leave frame");
    frames.extend(
        encode_frame_to_vec(&ClientMsg::hello(
            Role::Bot,
            "back",
            Some(KEYS[0]),
            bota_core::WIRE_FINGERPRINT,
        ))
        .expect("hello frame"),
    );
    leaver.stream.write_all(&frames).expect("leave then hello");
    while let Some(msg) = leaver.recv_or_hangup() {
        assert!(
            !matches!(
                msg,
                ServerMsg::Welcome { .. } | ServerMsg::MatchStart { .. }
            ),
            "{msg:?}"
        );
    }
    while let Some(msg) = stayer.recv_or_hangup() {
        assert!(!matches!(msg, ServerMsg::MatchStart { .. }), "{msg:?}");
    }
    assert_eq!(server.end(), MatchEnd::Abandoned);
}

fn a_disconnected_seat_can_rejoin_during_picks_or_a_running_match(wire: Wire) {
    for reserved_seats in [true, false] {
        for started in [false, true] {
            let (addr, mut clients) = three_seats(wire, reserved_seats, started);
            clients.remove(0).leave();
            let mut back = TestClient::connect(addr.as_str());
            back.hello(Role::Bot, Some(KEYS[0]));
            assert_eq!(back.welcome(), Some(SlotId(0)));
            if started {
                assert_eq!(back.match_start().picks[0].hero, HeroId::SHADOW_FIEND);
            } else {
                back.recv_until(|msg| match msg {
                    ServerMsg::LobbyState { slots, .. } => {
                        assert_eq!(slots[0].hero, Some(HeroId::SHADOW_FIEND));
                        Some(())
                    }
                    _ => None,
                });
            }
            back.say_all_and_hear("back after disconnect");
        }
    }
}

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
    // Both seats give orders; the watcher is told the one seat's alone, each as it is taken.
    let mut seen = Vec::new();
    for seq in 1..=3 {
        first.send(&move_order(seq, 1000, 1000 + seq as i32));
        second.send(&move_order(seq, 2000, 2000 + seq as i32));
        while seen.len() < seq as usize {
            let orders = watcher.recv_until(|msg| match msg {
                ServerMsg::Orders { orders, .. } => Some(orders),
                _ => None,
            });
            seen.extend(orders);
        }
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

    // Once the server has dealt with the departure, a third watcher takes the place. The
    // player is told nothing of it: players are not told about spectators.
    first.leave();
    let mut third = watch(KEYS[0]);
    assert_eq!(
        third.welcome(),
        None,
        "the departed watcher's place is taken"
    );
    player.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    let told = player.collect_until(
        |msg| matches!(msg, ServerMsg::LobbyState { slots, .. } if slots[0].hero.is_some()),
    );
    assert!(
        !told
            .iter()
            .any(|msg| matches!(msg, ServerMsg::ParticipantLeft { .. })),
        "{told:?}"
    );
}

#[test]
fn a_referee_hears_the_lobby_and_the_end_but_no_ticks_of_a_match_nobody_joined() {
    let mut opts = reserved(8);
    opts.mode = TickMode::Lockstep;
    opts.map = MapId(2);
    // The lobby ends this long after the referee has been welcomed: the clock is held until then.
    opts.start_after = Some(Duration::from_millis(300));
    let (addr, start) = start_gated_server(opts);
    let mut referee = TestClient::connect(addr);
    // A referee is told no tick, so nothing it hears shows the match advancing, and nobody
    // plays: the server simulates all of Map 2's twenty minutes at the pace of the CPU, a debug
    // build on a busy machine included.
    referee
        .stream
        .set_read_timeout(Some(HANG_GUARD * 6))
        .expect("timeout");
    referee.hello(Role::Referee, Some(REFEREE));
    assert_eq!(referee.welcome(), None);
    start.send(()).expect("the server runs");

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

/// A lobby started below its seats: one reserved seat on Map2 and nobody against it. The match
/// begins with that one player, and the empty side is not counted lost: nothing ends it.
#[test]
fn a_lone_reserved_seat_plays_on_with_the_other_side_empty() {
    let mut opts = reserved(22);
    opts.reserved.truncate(1);
    opts.referee_key = None;
    opts.map = MapId(2);
    let addr = start_server(opts);
    let mut seat = TestClient::connect(addr);
    seat.hello(Role::Bot, Some(KEYS[0]));
    seat.welcome();
    seat.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    seat.send(&ClientMsg::SetReady(true));
    let view = seat.recv_until(|msg| snapshot(msg).filter(|view| view.tick > 1));
    assert_eq!(view.players.len(), 1, "one player, no dummy");
    // Nothing ends it: a second of play later it still runs.
    let mut last = view.tick;
    for _ in 0..40 {
        match seat.recv_or_hangup() {
            Some(msg @ ServerMsg::MatchOver { .. }) => panic!("ended after tick {last}: {msg:?}"),
            Some(msg) => last = snapshot(msg.clone()).map_or(last, |view| view.tick),
            None => panic!("the server hung up after tick {last}"),
        }
    }
}

/// A watcher holding a seat's key that waits in the lobby is sent the match's first tick, the
/// state it starts from, however slowly it reads: the opening holds its connection's writer while
/// the seats play on, and the first tick is never folded into a later one.
#[test]
fn a_keyed_watcher_from_the_lobby_is_sent_the_first_tick_however_slowly_it_reads() {
    let mut opts = reserved(31);
    opts.mode = TickMode::Lockstep;
    opts.ack_timeout_ticks = 600; // twenty seconds at 30 a second: only acks move this match
    let addr = start_server(opts);
    // A receive window far below the match's opening, set before the connection opens so the
    // window is never offered wider: the server's writer is held on the opening while the
    // watcher reads nothing.
    let socket =
        socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).expect("a socket");
    socket.set_recv_buffer_size(1024).expect("receive buffer");
    socket.connect(&addr.into()).expect("connect");
    let stream = TcpStream::from(socket);
    stream
        .set_read_timeout(Some(HANG_GUARD))
        .expect("set timeout");
    let mut watcher = TestClient {
        stream,
        reader: Inbox::default(),
    };
    watcher.hello(Role::Spectator, Some(KEYS[0]));
    assert_eq!(watcher.welcome(), None);
    let mut seats: Vec<TestClient> = KEYS
        .iter()
        .map(|key| {
            let mut seat = TestClient::connect(addr);
            seat.hello(Role::Bot, Some(*key));
            seat.welcome();
            seat.send(&ClientMsg::PickHero {
                hero: HeroId::SHADOW_FIEND,
            });
            seat.send(&ClientMsg::SetReady(true));
            seat
        })
        .collect();
    // The seats play five ticks while the watcher still reads nothing.
    for _ in 0..5 {
        for seat in &mut seats {
            let tick = seat.recv_until(snapshot).tick;
            seat.send(&ClientMsg::Ack { tick });
        }
    }
    let info = watcher.match_start();
    let opening = encode_frame_to_vec(&ServerMsg::MatchStart { info }).expect("encodes");
    // The kernel keeps twice the send buffer asked for; past that, and the watcher's least
    // receive window, the writer was held on the opening.
    assert!(
        opening.len() > 2 * crate::net::SEND_BUFFER_BYTES,
        "the opening outgrows the server's send buffer"
    );
    let first = watcher.recv_until(snapshot);
    assert_eq!(
        (first.tick, first.skipped_ticks.len()),
        (1, 0),
        "the watcher starts from the first tick"
    );
}
