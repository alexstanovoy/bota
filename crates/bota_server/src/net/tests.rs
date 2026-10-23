//! The server speaks the protocol end to end over a real socket.

#[macro_use]
#[path = "../../tests/wire/mod.rs"]
mod wire;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{
    ClientMsg, EventKind, FrameReader, HeroId, Order, RejectReason, ReplayHeader, ReplayRecord,
    Role, ServerMsg, SlotId, Target, Team, Vec2, WIRE_FINGERPRINT, WorldView, encode_frame_to_vec,
};

use crate::game::OrderDelay;
use crate::game_loop::{
    DEFAULT_IDLE_TIMEOUT, DEFAULT_SPECTATORS_PER_SEAT, HELLO_DEADLINE, ServerOpts, run,
};
use crate::net::{DEFAULT_MESSAGES_PER_SEC, Listeners};
use crate::spectators::Spectators;
use wire::{Wire, dial};

struct TestClient {
    stream: TcpStream,
    reader: FrameReader,
}

impl TestClient {
    fn connect(addr: impl std::net::ToSocketAddrs) -> TestClient {
        let stream = TcpStream::connect(addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("set timeout");
        TestClient {
            stream,
            reader: FrameReader::new(),
        }
    }

    fn send(&mut self, msg: &ClientMsg) {
        let frame = encode_frame_to_vec(msg).expect("encode");
        self.stream.write_all(&frame).expect("send");
    }

    /// The next message; a ping is answered, as every client does, and skipped.
    fn recv(&mut self) -> ServerMsg {
        let mut buf = [0u8; 65536];
        loop {
            match self.reader.next_message::<ServerMsg>().expect("decode") {
                Some(ServerMsg::Ping) => self.send(&ClientMsg::Pong),
                Some(msg) => return msg,
                None => {}
            }
            let n = self.stream.read(&mut buf).expect("read");
            assert!(n > 0, "server hung up while a message was awaited");
            self.reader.push(&buf[..n]);
        }
    }

    fn recv_until<T>(&mut self, mut pick: impl FnMut(ServerMsg) -> Option<T>) -> T {
        for _ in 0..300 {
            if let Some(found) = pick(self.recv()) {
                return found;
            }
        }
        panic!("the awaited message never came");
    }
}

fn start_server(opts: ServerOpts) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || run(listener, opts));
    addr
}

/// Starts a server on free ports and returns where a TCP client dials it
/// over `wire`.
fn serve_over(opts: ServerOpts, wire: Wire) -> String {
    let tcp = TcpListener::bind("127.0.0.1:0").expect("bind");
    let (listeners, addr): (Listeners, _) = match wire {
        Wire::Tcp => {
            let addr = tcp.local_addr().expect("addr");
            (tcp.into(), addr)
        }
        #[cfg(feature = "websocket")]
        Wire::Ws | Wire::Wss => {
            use crate::net::{Tls, WsListener};
            use std::path::Path;
            let ws = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = ws.local_addr().expect("addr");
            let tls = (wire == Wire::Wss).then(|| {
                Tls::from_pem_files(Path::new(wire::CERT), Path::new(wire::KEY))
                    .expect("the test TLS")
            });
            (
                Listeners::with_websocket(tcp, WsListener::new(ws, tls)),
                addr,
            )
        }
    };
    thread::spawn(move || run(listeners, opts));
    dial(wire, &addr.to_string())
}

fn join_as_bot(addr: std::net::SocketAddr, name: &str) -> (TestClient, Option<SlotId>) {
    let mut client = TestClient::connect(addr);
    client.send(&ClientMsg::hello(Role::Bot, name, None));
    let slot = client.recv_until(|msg| match msg {
        ServerMsg::Welcome { slot, .. } => Some(slot),
        _ => None,
    });
    client.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    client.send(&ClientMsg::SetReady(true));
    (client, slot)
}

/// A realtime two-seat match on Map0 with every option off.
fn realtime(seed: u64) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        mode: bota_proto::TickMode::Realtime,
        tick_rate: 60,
        players: 2,
        replay: None,
        seed: Some(seed),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: bota_proto::MapId(0),
        ack_timeout_ticks: 150,
        idle_timeout: Some(DEFAULT_IDLE_TIMEOUT),
        cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        dummy_seat: false,
        order_delays: Vec::new(),
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: None,
    }
}

fn move_order(seq: u32, x: i32, y: i32) -> ClientMsg {
    ClientMsg::Order {
        seq,
        unit: None,
        order: Order::Move {
            target: Target::Pos(Vec2::from_ints(x, y)),
        },
        seen: None,
    }
}

fn snapshot(msg: ServerMsg) -> Option<WorldView> {
    match msg {
        ServerMsg::Snapshot { view } => Some(view),
        _ => None,
    }
}

fn match_start_picks(msg: ServerMsg) -> Option<usize> {
    match msg {
        ServerMsg::MatchStart { info } => Some(info.picks.len()),
        _ => None,
    }
}

#[test]
fn a_client_built_with_another_wire_is_told_so_and_takes_no_seat() {
    let addr = start_server(realtime(5));
    let mut stranger = TestClient::connect(addr);
    stranger.send(&ClientMsg::Hello {
        role: Role::Bot,
        name: "stranger".to_string(),
        key: None,
        wire_fingerprint: !WIRE_FINGERPRINT,
    });
    assert_eq!(
        stranger.recv(),
        ServerMsg::WireMismatch {
            server_fingerprint: WIRE_FINGERPRINT
        }
    );
    let mut rest = Vec::new();
    stranger
        .stream
        .read_to_end(&mut rest)
        .expect("the server hangs up");
    assert!(rest.is_empty(), "nothing follows the refusal");
    let (_client, slot) = join_as_bot(addr, "native");
    assert_eq!(slot, Some(SlotId(0)), "the refused peer held no seat");
}

#[test]
fn a_long_name_is_shown_cut_at_a_whole_character() {
    let addr = start_server(realtime(5));
    // Three bytes a character: 32 bytes hold ten whole ones.
    let name = "\u{20ac}".repeat(20);
    let (mut client, slot) = join_as_bot(addr, &name);
    let shown = client.recv_until(|msg| match msg {
        ServerMsg::LobbyState { slots, .. } => slots
            .into_iter()
            .find(|row| Some(row.slot) == slot && !row.name.is_empty())
            .map(|row| row.name),
        _ => None,
    });
    assert_eq!(bota_core::rules::MAX_SHOWN_NAME_BYTES, 32);
    assert_eq!(shown, "\u{20ac}".repeat(10));
}

#[test]
fn a_realtime_match_reaches_two_clients() {
    let replay_path = std::env::temp_dir().join("bota-net-test-realtime.brp");
    let mut opts = realtime(7);
    opts.replay = Some(replay_path.clone());
    let addr = start_server(opts);

    let (mut c1, slot1) = join_as_bot(addr, "alpha");
    let (mut c2, slot2) = join_as_bot(addr, "beta");
    assert_eq!(slot1, Some(SlotId(0)));
    assert_eq!(slot2, Some(SlotId(1)));
    for client in [&mut c1, &mut c2] {
        assert_eq!(client.recv_until(match_start_picks), 2);
    }

    let first = c1.recv_until(snapshot);
    assert_eq!(first.viewer, Some(Team::Radiant));
    assert_eq!(first.players.len(), 2);
    assert!(first.units.iter().any(|u| u.team == Team::Radiant));
    let first_events = c1.recv_until(|msg| match msg {
        ServerMsg::Events { tick, events } if tick == first.tick => Some(events),
        _ => None,
    });
    assert!(first_events.is_empty());

    // A legal order passes silently; an impossible one is named and refused.
    c1.send(&move_order(1, 1000, 1000));
    c1.send(&ClientMsg::Order {
        seq: 2,
        unit: None,
        order: Order::Buy {
            item: bota_proto::ItemId(999),
        },
        seen: None,
    });
    let (seq, reason) = c1.recv_until(|msg| match msg {
        ServerMsg::OrderRejected { seq, reason } => Some((seq, reason)),
        _ => None,
    });
    assert_eq!(seq, 2);
    assert_eq!(reason, RejectReason::UnknownItem);
    let later = c1.recv_until(|msg| snapshot(msg).filter(|view| view.tick > first.tick));
    assert!(later.tick > first.tick);

    // Hanging up empties the seats; the server ends the match and flushes:
    // the replay names the wire it was recorded with, then the match.
    drop(c1);
    drop(c2);
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(50));
        let bytes = std::fs::read(&replay_path).unwrap_or_default();
        let mut reader = FrameReader::new();
        reader.push(&bytes);
        let header = reader.next_message::<ReplayRecord>();
        if let Ok(Some(ReplayRecord::Msg(ServerMsg::MatchStart { .. }))) =
            reader.next_message::<ReplayRecord>()
        {
            assert_eq!(
                header,
                Ok(Some(ReplayRecord::Header(ReplayHeader::CURRENT)))
            );
            return;
        }
    }
    panic!("the replay never became readable");
}

#[test]
fn lockstep_advances_as_fast_as_the_acks_come() {
    let mut opts = realtime(8);
    opts.mode = bota_proto::TickMode::Lockstep;
    opts.ack_timeout_ticks = 600; // ten seconds: only acks can move this match
    let addr = start_server(opts);

    let (mut c1, _) = join_as_bot(addr, "alpha");
    let (mut c2, _) = join_as_bot(addr, "beta");

    // Both ack every snapshot; five ticks must pass well inside the timeout.
    let mut last_tick = 0;
    for _ in 0..5 {
        for client in [&mut c1, &mut c2] {
            let tick = client.recv_until(snapshot).tick;
            client.send(&ClientMsg::Ack { tick });
            last_tick = last_tick.max(tick);
        }
    }
    assert!(last_tick >= 4, "the match advanced on acknowledgements");
}

#[test]
fn a_dummy_seat_lets_one_client_start_a_match_alone() {
    let mut opts = realtime(11);
    opts.map = bota_proto::MapId(2);
    opts.immortal_structures = true;
    opts.dummy_seat = true;
    let addr = start_server(opts);
    let (mut client, slot) = join_as_bot(addr, "alone");
    assert_eq!(slot, Some(SlotId(0)));
    assert_eq!(client.recv_until(match_start_picks), 2);
    let view = client.recv_until(|msg| snapshot(msg).filter(|view| view.tick > 1));
    assert_eq!(view.players.len(), 2);
}

#[test]
fn a_connection_that_has_not_said_hello_is_told_no_tick() {
    let mut opts = realtime(12);
    opts.map = bota_proto::MapId(2);
    opts.dummy_seat = true;
    // The stranger never answers a ping; what it is told is the point here, not how long it is
    // kept.
    opts.idle_timeout = None;
    let addr = start_server(opts);
    let (mut client, _) = join_as_bot(addr, "seated");
    client.recv_until(snapshot);
    let mut silent = TestClient::connect(addr);
    // Its writer pings it once a second and sends whatever is queued between two pings, so two
    // pings mean a second of ticks would have reached it, had it been sent any.
    let mut pings = 0;
    let mut buf = [0u8; 65536];
    while pings < 2 {
        let n = silent.stream.read(&mut buf).expect("the server pings");
        assert!(n > 0, "the server hung up");
        silent.reader.push(&buf[..n]);
        while let Some(msg) = silent.reader.next_message::<ServerMsg>().expect("decode") {
            assert!(
                !matches!(msg, ServerMsg::Snapshot { .. } | ServerMsg::Events { .. }),
                "a stranger was told the fogless world"
            );
            pings += usize::from(msg == ServerMsg::Ping);
        }
    }
}

#[test]
fn a_stranger_that_never_says_hello_is_let_go_and_frees_its_place() {
    let mut opts = realtime(13);
    opts.max_connections = Some(1);
    let addr = start_server(opts);
    let mut stranger = TestClient::connect(addr);
    let started = Instant::now();
    // It answers every ping, as a live client does, and never says Hello.
    let pong = encode_frame_to_vec(&ClientMsg::Pong).expect("encode");
    let mut buf = [0u8; 65536];
    while let Ok(n @ 1..) = stranger.stream.read(&mut buf) {
        assert!(
            started.elapsed() < HELLO_DEADLINE + Duration::from_secs(5),
            "the stranger is kept"
        );
        stranger.reader.push(&buf[..n]);
        while let Some(msg) = stranger.reader.next_message::<ServerMsg>().expect("decode") {
            if msg == ServerMsg::Ping {
                let _ = stranger.stream.write_all(&pong);
            }
        }
    }
    assert!(started.elapsed() >= HELLO_DEADLINE - Duration::from_secs(1));
    let (_seated, slot) = join_as_bot(addr, "late");
    assert_eq!(slot, Some(SlotId(0)));
}

#[test]
fn a_delayed_order_takes_effect_late_and_its_delay_is_told() {
    const DELAY: u32 = 3;
    let mut opts = realtime(12);
    opts.mode = bota_proto::TickMode::Lockstep;
    opts.ack_timeout_ticks = 600;
    opts.dummy_seat = true;
    opts.order_delays = vec![OrderDelay {
        base: DELAY,
        jitter: 0,
    }];
    let addr = start_server(opts);
    let (mut client, slot) = join_as_bot(addr, "late");
    let slot = slot.expect("a seat");
    let hero_of = |view: &WorldView| {
        let unit = view.players.iter().find(|p| p.slot == slot)?.unit?;
        let hero = view.units.iter().find(|u| u.id == unit)?;
        Some((hero.pos, hero.facing))
    };
    let first = client.recv_until(snapshot);
    let at_rest = hero_of(&first).expect("the hero stands");
    client.send(&ClientMsg::Order {
        seq: 1,
        unit: None,
        order: Order::Move {
            target: Target::Pos(Vec2::from_ints(-2000, 3000)),
        },
        seen: Some(first.tick),
    });
    client.send(&ClientMsg::Ack { tick: first.tick });
    let due = first.tick + 1 + DELAY;
    loop {
        let view = client.recv_until(snapshot);
        client.send(&ClientMsg::Ack { tick: view.tick });
        if view.tick < due {
            assert_eq!(hero_of(&view), Some(at_rest), "tick {}", view.tick);
            assert!(view.order_delays.is_empty());
            continue;
        }
        assert_ne!(hero_of(&view), Some(at_rest));
        assert_eq!(
            view.order_delays,
            vec![bota_proto::OrderDelayView { slot, ticks: DELAY }]
        );
        return;
    }
}

/// A snapshot of `tick` some 3.5 MB long, more than the socket buffers hold.
fn bulky_snapshot(tick: u32) -> Vec<u8> {
    let view = WorldView {
        tick,
        viewer: Some(Team::Radiant),
        units: Vec::new(),
        projectiles: Vec::new(),
        players: Vec::new(),
        felled_trees: Vec::new(),
        planted_trees: vec![Vec2::from_ints(7_000, 7_000); 350_000],
        loot: Vec::new(),
        aggro: Vec::new(),
        order_delays: Vec::new(),
        skipped_ticks: Vec::new(),
        stock: Vec::new(),
        talents: Vec::new(),
        teleports: Vec::new(),
        glyphs: Vec::new(),
    };
    encode_frame_to_vec(&ServerMsg::Snapshot { view }).expect("under the frame limit")
}

#[test]
fn a_slow_reader_skips_snapshots_but_no_tick_and_no_event() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let mut client = TestClient::connect(listener.local_addr().expect("addr"));
    let (stream, _) = listener.accept().expect("accept");
    // Buffers far below one snapshot, whatever the kernel would grow them to:
    // a writer that took a snapshot is held there while the peer reads nothing.
    socket2::SockRef::from(&stream)
        .set_send_buffer_size(crate::net::SEND_BUFFER_BYTES)
        .expect("send buffer");
    socket2::SockRef::from(&client.stream)
        .set_recv_buffer_size(64 * 1024)
        .expect("receive buffer");
    // Held open until the end: closed with the client's pongs unread, the
    // socket would reset the connection under the client.
    let held = stream.try_clone().expect("clone");
    let outbox = crate::net::Outbox::new();
    let writer = {
        let outbox = std::sync::Arc::clone(&outbox);
        thread::spawn(move || outbox.run_writer(Box::new(stream)))
    };
    let events = |tick: u32| {
        vec![EventKind::LevelUp {
            unit: bota_proto::EntityId {
                idx: tick,
                generation: 1,
            },
            level: 2,
        }]
    };
    // The writer takes the first tick's snapshot and blocks on it while the peer reads
    // nothing, so the later ticks coalesce into the last one.
    outbox.push_tick(bulky_snapshot(1), 1, events(1));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !outbox.is_empty() {
        assert!(Instant::now() < deadline, "the writer takes the first tick");
        thread::sleep(Duration::from_millis(5));
    }
    for tick in 2..=6 {
        outbox.push_tick(bulky_snapshot(tick), tick, events(tick));
    }
    outbox.close();
    let mut seen = Vec::new();
    loop {
        match client.recv() {
            ServerMsg::Snapshot { view } => seen.push((
                view.tick,
                view.skipped_ticks
                    .into_iter()
                    .map(|skipped| (skipped.tick, skipped.events))
                    .collect::<Vec<_>>(),
            )),
            ServerMsg::Events { tick, events: told } => {
                assert_eq!(seen.last().map(|(at, _)| *at), Some(tick));
                assert_eq!(told, events(tick));
                if tick == 6 {
                    break;
                }
            }
            ServerMsg::Ping => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    writer.join().expect("writer");
    drop(held);
    let skipped: Vec<(u32, Vec<EventKind>)> = (2..=5).map(|tick| (tick, events(tick))).collect();
    assert_eq!(seen, vec![(1, Vec::new()), (6, skipped)]);
}

/// Reads snapshots and events up to the snapshot of tick `until`, checking that every
/// tick between two snapshots is told once, with its events after its own snapshot;
/// returns the last tick, the pending events tick and how many ticks were skipped.
fn read_ticks(
    client: &mut TestClient,
    (mut last, mut events_tick): (u32, Option<u32>),
    until: u32,
) -> (u32, Option<u32>, usize) {
    let mut skipped = 0;
    while last < until {
        match client.recv() {
            ServerMsg::Snapshot { view } => {
                assert_eq!(events_tick, None, "the previous tick's events came");
                let told: Vec<u32> = view.skipped_ticks.iter().map(|s| s.tick).collect();
                assert_eq!(told, (last + 1..view.tick).collect::<Vec<_>>());
                skipped += told.len();
                last = view.tick;
                events_tick = Some(view.tick);
            }
            ServerMsg::Events { tick, .. } => {
                assert_eq!(
                    events_tick.take(),
                    Some(tick),
                    "events follow their snapshot"
                );
            }
            _ => {}
        }
    }
    (last, events_tick, skipped)
}

#[test]
fn a_reader_that_keeps_up_is_never_skipped_and_a_stalled_one_is_at_once() {
    let mut opts = realtime(14);
    // A tick every 100 ms: a reader that keeps up stays inside one tick however
    // loaded the machine is.
    opts.tick_rate = 10;
    opts.map = bota_proto::MapId(2);
    // The stall below outlasts the default; liveness is not under test here.
    opts.idle_timeout = Some(Duration::from_secs(30));
    let addr = start_server(opts);
    let mut lagging = TestClient::connect(addr);
    // A small receive buffer: a stalled reader's backlog outgrows the buffers
    // in a few dozen snapshots.
    socket2::SockRef::from(&lagging.stream)
        .set_recv_buffer_size(16 * 1024)
        .expect("receive buffer");
    lagging.send(&ClientMsg::hello(Role::Bot, "lagging", None));
    lagging.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    lagging.send(&ClientMsg::SetReady(true));
    // The other seat reads all along and tells the time.
    let (mut clock, _) = join_as_bot(addr, "clock");
    let first = lagging.recv_until(snapshot).tick;
    let (last, pending, skipped) = read_ticks(&mut lagging, (first, Some(first)), first + 20);
    assert_eq!(skipped, 0, "a reader that keeps up sees every snapshot");
    // Sixty ticks go by while the lagging reader reads nothing.
    let stalled_at = clock.recv_until(snapshot).tick;
    while clock.recv_until(snapshot).tick < stalled_at + 60 {}
    let (_, _, skipped) = read_ticks(&mut lagging, (last, pending), stalled_at + 60);
    assert!(skipped > 0, "a stalled reader is skipped ahead");
}

mod pick_clock;
mod picks;
mod reserved;
mod spectators;
