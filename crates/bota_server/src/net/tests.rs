//! The server speaks the protocol end to end over a real socket.

#[macro_use]
#[path = "../../tests/wire/mod.rs"]
mod wire;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bota_proto::{
    Audience, Body, ClientMsg, EventKind, FrameReader, HeroId, Line, Order, RejectReason,
    ReplayHeader, ReplayRecord, Role, ServerMsg, SlotId, Target, Team, Vec2, WorldView,
    encode_frame_to_vec,
};

use crate::game::OrderDelay;
use crate::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, HELLO_DEADLINE, MatchEnd, ServerOpts, run};
use crate::net::{DEFAULT_MESSAGES_PER_SEC, Listeners};
use crate::spectators::Spectators;
use wire::{Wire, dial};

/// How long a test waits for something the server does on its own, such as a message it owes or
/// the end of a match. It guards against a hang and nothing else: a healthy server, however busy
/// the machine, answers long before it, and no test asserts how long an answer took. Waits in
/// tests are for the message or state itself, never for a span to pass.
const HANG_GUARD: Duration = Duration::from_secs(300);

struct TestClient {
    stream: TcpStream,
    reader: Inbox,
}

/// What a test client has read and not yet taken: frames off the socket,
/// compressed parts unpacked as a client library does.
#[derive(Default)]
struct Inbox {
    frames: FrameReader,
    unpacker: bota_proto::Unpacker,
}

impl Inbox {
    fn push(&mut self, bytes: &[u8]) {
        self.frames.push(bytes);
    }

    fn next_message(&mut self) -> Result<Option<ServerMsg>, bota_proto::CodecError> {
        loop {
            if let Some(msg) = self.unpacker.next_message()? {
                return Ok(Some(msg));
            }
            let Some(msg) = self.frames.next_message::<ServerMsg>()? else {
                return Ok(None);
            };
            if let Some(msg) = self.unpacker.take(msg)? {
                return Ok(Some(msg));
            }
        }
    }
}

impl TestClient {
    fn connect(addr: impl std::net::ToSocketAddrs) -> TestClient {
        let stream = TcpStream::connect(addr).expect("connect");
        stream
            .set_read_timeout(Some(HANG_GUARD))
            .expect("set timeout");
        TestClient {
            stream,
            reader: Inbox::default(),
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
            match self.reader.next_message().expect("decode") {
                Some(ServerMsg::Ping) => self.send(&ClientMsg::Pong),
                Some(msg) => return msg,
                None => {}
            }
            let n = self.stream.read(&mut buf).expect("read");
            assert!(n > 0, "server hung up while a message was awaited");
            self.reader.push(&buf[..n]);
        }
    }

    /// Reads on until `pick` takes a message. However many messages come first, as a busy
    /// machine lets the server get ahead of the reader by any number: the read timeout
    /// ([`HANG_GUARD`]) ends a wait for what never comes.
    fn recv_until<T>(&mut self, mut pick: impl FnMut(ServerMsg) -> Option<T>) -> T {
        loop {
            if let Some(found) = pick(self.recv()) {
                return found;
            }
        }
    }

    /// Everything the server tells this client up to and with the first message `last` accepts
    /// (pings answered, not kept). The server tells a client its messages in the order it queued
    /// them, so a message queued after an event can mark the point by which that event, had the
    /// client been owed it, has been told.
    fn collect_until(&mut self, mut last: impl FnMut(&ServerMsg) -> bool) -> Vec<ServerMsg> {
        let mut told = Vec::new();
        loop {
            let msg = self.recv();
            let done = last(&msg);
            told.push(msg);
            if done {
                return told;
            }
        }
    }

    /// Everything told up to and with the first snapshot of `tick` or later.
    fn collect_through_tick(&mut self, tick: u32) -> Vec<ServerMsg> {
        self.collect_until(|msg| matches!(msg, ServerMsg::Snapshot { view } if view.tick >= tick))
    }

    /// The tick of the next snapshot. Read after the last message a test waited for, it is a tick
    /// published after the server dealt with whatever caused that message, so a snapshot of that
    /// tick or later, in any client's stream, comes after everything the server queued for that
    /// client because of it.
    fn next_tick(&mut self) -> u32 {
        self.recv_until(snapshot).tick
    }

    /// Says `text` to all and reads on until the server gives it back. Everything this client
    /// sent before has reached the server by then, and so has what other clients had been
    /// told to have sent; what is queued for the others afterwards comes after the line.
    fn say_all_and_hear(&mut self, text: &str) -> Vec<ServerMsg> {
        self.send(&say(Audience::All, text));
        self.collect_until(|msg| matches!(msg, ServerMsg::Heard(heard) if heard.line.body == Body::Text(text.to_owned())))
    }

    /// Closes its end of the connection, as a client that leaves does, and reads on until the
    /// server hangs up too. By then the server has dealt with the departure: what it tells
    /// the others of it is queued before anything it queues afterwards.
    fn leave(mut self) {
        self.stream
            .shutdown(std::net::Shutdown::Write)
            .expect("close the sending end");
        let mut buf = [0u8; 65536];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return,
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => return,
                Err(error) => panic!("the server did not hang up on a leaver: {error}"),
            }
        }
    }
}

fn say(to: Audience, text: &str) -> ClientMsg {
    ClientMsg::Say(Line {
        to,
        body: Body::Text(text.to_owned()),
    })
}

/// A server running on its own thread: where to dial it, and a wait for its end.
struct RunningServer {
    addr: std::net::SocketAddr,
    ended: mpsc::Receiver<std::io::Result<MatchEnd>>,
}

impl RunningServer {
    /// Waits for the server to end the match and finish everything it writes, the replay too.
    fn end(self) -> MatchEnd {
        self.ended
            .recv_timeout(HANG_GUARD)
            .expect("the server ends the match")
            .expect("a clean end")
    }
}

/// A replay file's frames, unpacked; nothing while the server has not
/// finished writing it.
fn read_replay(path: &std::path::Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    bota_client_core::UnpackedReplay::new(file)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

fn start_server(opts: ServerOpts) -> std::net::SocketAddr {
    start_ending_server(opts).addr
}

/// Like [`start_server`], but the lobby's clock ([`ServerOpts::start_after`]) runs only once the
/// returned sender is sent to: the test sends it when the connections it is about have been
/// admitted, so the lobby cannot end before they are in.
fn start_gated_server(opts: ServerOpts) -> (std::net::SocketAddr, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (open, gate) = mpsc::channel();
    thread::spawn(move || {
        let _ = crate::game_loop::run_gated(listener, opts, gate);
    });
    (addr, open)
}

/// Like [`start_server`], for a test that waits for the match to end.
fn start_ending_server(opts: ServerOpts) -> RunningServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (done, ended) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(run(listener, opts));
    });
    RunningServer { addr, ended }
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
    client.send(&ClientMsg::hello(
        Role::Bot,
        name,
        None,
        bota_core::WIRE_FINGERPRINT,
    ));
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

/// A realtime two-seat match on Map0 with every option off. Nobody is let go for silence: the
/// test works on one client while the others sit unread, and on a busy machine that takes long
/// enough to run out any idle time and cost a seat. The tick rate is low for the same reason: the
/// server cuts off a client that has skipped 900 ticks in a row, so a client left unread for
/// fifteen seconds at 60 a second would go, and at 20 it keeps its place for three-quarters of a minute.
fn realtime(seed: u64) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: crate::chat::ChatOpts::DEFAULT,
        mode: bota_proto::TickMode::Realtime,
        tick_rate: 20,
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
        idle_timeout: None,
        cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: false,
        seated_start: false,
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
        queue: false,
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

/// A client of other registries, or of the same registries and other rules
/// (a balance change), is refused before it could misread anything.
#[test]
fn a_client_built_with_another_wire_or_other_rules_is_told_so_and_takes_no_seat() {
    use bota_core::{RULES_VERSION, WIRE_FINGERPRINT};
    let other_rules = bota_proto::fnv1a(&[
        &bota_proto::REGISTRY_DIGEST.to_le_bytes(),
        &(RULES_VERSION + 1).to_le_bytes(),
    ]);
    let addr = start_server(realtime(5));
    for theirs in [!WIRE_FINGERPRINT, other_rules] {
        let mut stranger = TestClient::connect(addr);
        stranger.send(&ClientMsg::Hello {
            role: Role::Bot,
            name: "stranger".to_string(),
            key: None,
            wire_fingerprint: theirs,
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
    }
    let (_client, slot) = join_as_bot(addr, "native");
    assert_eq!(slot, Some(SlotId(0)), "the refused peers held no seat");
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
    let replay_path =
        std::env::temp_dir().join(format!("bota-net-test-realtime-{}.brp", std::process::id()));
    let mut opts = realtime(7);
    opts.replay = Some(replay_path.clone());
    let server = start_ending_server(opts);
    let addr = server.addr;

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
        queue: false,
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
    server.end();
    let bytes = read_replay(&replay_path).expect("the replay is finished");
    let _ = std::fs::remove_file(&replay_path);
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    assert_eq!(
        reader.next_message::<ReplayRecord>(),
        Ok(Some(ReplayRecord::Header(ReplayHeader::new(
            bota_core::WIRE_FINGERPRINT
        ))))
    );
    assert!(matches!(
        reader.next_message::<ReplayRecord>(),
        Ok(Some(ReplayRecord::Msg(ServerMsg::MatchStart { .. })))
    ));
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
    // kept: no test of this module lets the server drop a silent client, see `realtime`.
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
        while let Some(msg) = silent.reader.next_message().expect("decode") {
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
        assert!(started.elapsed() < HANG_GUARD, "the stranger is kept");
        stranger.reader.push(&buf[..n]);
        while let Some(msg) = stranger.reader.next_message().expect("decode") {
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
        queue: false,
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

/// A snapshot of `tick` some 200 KB long (every collection that holds no unit at its bound), more
/// than the socket buffers of the tests that use it hold.
fn bulky_snapshot(tick: u32) -> Vec<u8> {
    let spot = Vec2::from_ints(7_000, 7_000);
    let missile = |idx: u32| bota_proto::ProjectileView {
        id: bota_proto::EntityId { idx, generation: 1 },
        pos: spot,
        facing: bota_proto::Angle::default(),
        team: Team::Radiant,
        ability: None,
        source: None,
        target: None,
        inflictor: None,
        seen_by: None,
        model: None,
    };
    let dropped = |idx: u32| bota_proto::LootView {
        id: bota_proto::EntityId { idx, generation: 1 },
        pos: spot,
        item: bota_proto::ItemId::HEALING_SALVE,
        charges: None,
        seen_by: None,
    };
    let view = WorldView {
        tick,
        viewer: Some(Team::Radiant),
        units: Vec::new(),
        projectiles: (0..bota_proto::MAX_PROJECTILES as u32)
            .map(missile)
            .collect(),
        players: Vec::new(),
        felled_trees: Vec::new(),
        planted_trees: vec![spot; bota_proto::MAX_PLANTED_TREES],
        loot: (0..bota_proto::MAX_SNAPSHOT_LOOT as u32)
            .map(dropped)
            .collect(),
        aggro: Vec::new(),
        order_delays: Vec::new(),
        skipped_ticks: Vec::new(),
        stock: Vec::new(),
        talents: Vec::new(),
        teleports: Vec::new(),
        glyphs: Vec::new(),
        clock: None,
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
        .set_recv_buffer_size(8 * 1024)
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
    let deadline = Instant::now() + HANG_GUARD;
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
    // A tick every half second, some 4.6 kB a tick: a reader that reads as the ticks come stays
    // inside one tick however loaded the machine is (a server thread that waits a whole tick for
    // the processor would skip it, and the machine's load never takes that long).
    opts.tick_rate = 2;
    opts.map = bota_proto::MapId(2);
    let addr = start_server(opts);
    let mut lagging = TestClient::connect(addr);
    // The smallest receive buffer: a stalled reader's backlog outgrows the buffers in a few
    // ticks.
    socket2::SockRef::from(&lagging.stream)
        .set_recv_buffer_size(4 * 1024)
        .expect("receive buffer");
    lagging.send(&ClientMsg::hello(
        Role::Bot,
        "lagging",
        None,
        bota_core::WIRE_FINGERPRINT,
    ));
    lagging.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    lagging.send(&ClientMsg::SetReady(true));
    // The other seat reads all along and tells the time.
    let (mut clock, _) = join_as_bot(addr, "clock");
    let first = lagging.recv_until(snapshot).tick;
    let (last, pending, skipped) = read_ticks(&mut lagging, (first, Some(first)), first + 8);
    assert_eq!(skipped, 0, "a reader that keeps up sees every snapshot");
    // Thirty ticks are played in all, the later twenty or so while the lagging reader reads nothing.
    let stalled_at = clock.recv_until(snapshot).tick;
    while clock.recv_until(snapshot).tick < stalled_at + 30 {}
    let (_, _, skipped) = read_ticks(&mut lagging, (last, pending), stalled_at + 30);
    assert!(skipped > 0, "a stalled reader is skipped ahead");
}

mod chat;
mod pick_clock;
mod picks;
mod reserved;
mod spectators;
