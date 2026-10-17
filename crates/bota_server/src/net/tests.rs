//! The server speaks the protocol end to end over a real socket.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use bota_proto::{
    ClientMsg, EventKind, FrameReader, HeroId, Order, RejectReason, ReplayRecord, Role, ServerMsg,
    SlotId, Target, Team, Vec2, WIRE_FINGERPRINT, WorldView, encode_frame_to_vec,
};

use crate::game::OrderDelay;
use crate::game_loop::{ServerOpts, run};

struct TestClient {
    stream: TcpStream,
    reader: FrameReader,
}

impl TestClient {
    fn connect(addr: std::net::SocketAddr) -> TestClient {
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

    fn recv(&mut self) -> ServerMsg {
        let mut buf = [0u8; 65536];
        loop {
            if let Some(msg) = self.reader.next_message::<ServerMsg>().expect("decode") {
                return msg;
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

fn join_as_bot(addr: std::net::SocketAddr, name: &str) -> (TestClient, Option<SlotId>) {
    let mut client = TestClient::connect(addr);
    client.send(&ClientMsg::Hello {
        role: Role::Bot,
        name: name.to_string(),
        key: None,
        wire_fingerprint: WIRE_FINGERPRINT,
    });
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
        cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        dummy_seat: false,
        order_delays: Vec::new(),
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

    // Hanging up empties the seats; the server ends the match and flushes.
    drop(c1);
    drop(c2);
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(50));
        let bytes = std::fs::read(&replay_path).unwrap_or_default();
        let mut reader = FrameReader::new();
        reader.push(&bytes);
        if let Ok(Some(ReplayRecord::Msg(ServerMsg::MatchStart { .. }))) =
            reader.next_message::<ReplayRecord>()
        {
            return;
        }
    }
    panic!("the replay never became readable");
}

#[test]
fn a_spectator_borrows_eyes_and_is_told_the_orders_they_see() {
    let addr = start_server(realtime(9));
    let (mut c1, _) = join_as_bot(addr, "alpha");
    let (mut c2, _) = join_as_bot(addr, "beta");
    let mut watcher = TestClient::connect(addr);
    watcher.send(&ClientMsg::Hello {
        role: Role::Spectator,
        name: "eye".to_string(),
        key: None,
        wire_fingerprint: WIRE_FINGERPRINT,
    });
    let slot = watcher.recv_until(|msg| match msg {
        ServerMsg::Welcome { slot, .. } => Some(slot),
        _ => None,
    });
    assert_eq!(slot, None, "a spectator takes no seat");
    let first = watcher.recv_until(snapshot);
    assert_eq!(
        first.viewer, None,
        "everything, before any eyes are asked for"
    );

    // An order given while nobody's eyes are borrowed is told to no spectator.
    c1.send(&move_order(1, 1000, 1000));
    let mut told_freely = 0u32;
    for _ in 0..3 {
        watcher.recv_until(|msg| match msg {
            ServerMsg::Orders { .. } => {
                told_freely += 1;
                None
            }
            ServerMsg::Snapshot { .. } => Some(()),
            _ => None,
        });
    }
    assert_eq!(told_freely, 0, "watching everything is told no orders");

    watcher.send(&ClientMsg::ViewAs {
        seat: Some(SlotId(1)),
    });
    let fogged = watcher.recv_until(|msg| snapshot(msg).filter(|view| view.viewer.is_some()));
    assert_eq!(
        fogged.viewer,
        Some(Team::Dire),
        "the snapshots come through the chosen eyes"
    );

    // Through one side's eyes, only that side's orders are told.
    c2.send(&move_order(1, 15000, 15000));
    let told = watcher.recv_until(|msg| match msg {
        ServerMsg::Orders { orders, .. } => Some(orders),
        _ => None,
    });
    assert!(
        told.iter().all(|given| given.slot == SlotId(1)),
        "an order of the side watched: {told:?}"
    );
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
    };
    encode_frame_to_vec(&ServerMsg::Snapshot { view }).expect("under the frame limit")
}

#[test]
fn a_slow_reader_skips_snapshots_but_no_tick_and_no_event() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let mut client = TestClient::connect(listener.local_addr().expect("addr"));
    let (stream, _) = listener.accept().expect("accept");
    let outbox = crate::net::Outbox::new();
    let writer = {
        let outbox = std::sync::Arc::clone(&outbox);
        thread::spawn(move || outbox.run_writer(stream))
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
    // The writer blocks on the first tick's snapshot while the peer reads nothing, so the
    // later ticks coalesce into the last one.
    for tick in 1..=6 {
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
            other => panic!("unexpected {other:?}"),
        }
    }
    writer.join().expect("writer");
    let skipped: Vec<(u32, Vec<EventKind>)> = (2..=5).map(|tick| (tick, events(tick))).collect();
    assert_eq!(seen, vec![(1, Vec::new()), (6, skipped)]);
}

/// Reads snapshots and events for `wall`, checking that every tick between two snapshots
/// is told once, with its events after its own snapshot; returns the last tick, the
/// pending events tick and how many ticks were skipped.
fn read_ticks(
    client: &mut TestClient,
    (mut last, mut events_tick): (u32, Option<u32>),
    wall: Duration,
) -> (u32, Option<u32>, usize) {
    let mut skipped = 0;
    let started = std::time::Instant::now();
    while started.elapsed() < wall {
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
    opts.tick_rate = 30;
    opts.map = bota_proto::MapId(2);
    opts.dummy_seat = true;
    let addr = start_server(opts);
    let (mut client, _) = join_as_bot(addr, "lagging");
    let first = client.recv_until(snapshot).tick;
    let (last, pending, skipped) =
        read_ticks(&mut client, (first, Some(first)), Duration::from_secs(2));
    assert_eq!(skipped, 0, "a reader that keeps up sees every snapshot");
    assert!(last >= first + 45);
    // Three seconds of Map2 snapshots outgrow the capped send buffer and the reader's own.
    thread::sleep(Duration::from_secs(3));
    let (_, _, skipped) = read_ticks(&mut client, (last, pending), Duration::from_secs(1));
    assert!(skipped > 0, "a stalled reader is skipped ahead");
}

mod reserved;
