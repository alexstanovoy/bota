//! The server and the client library over bad lines: high latency and
//! jitter, a bandwidth cap, stalls, resets and a black hole, made by an
//! in-process proxy between them. Every test runs over TCP, and with the
//! `websocket` feature over a WebSocket, plain and over TLS.

mod proxy;
#[macro_use]
mod wire;

use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use bota_client_core::{
    ClientSession, DEFAULT_IO_TIMEOUT, Endpoint, Ingested, IssuedOrder, Link, Net, Outcome, Redial,
    Seat, Seated, SessionOptions, Transport, join,
};
use bota_core::fountain_pos;
use bota_proto::{ClientMsg, HeroId, MapId, Order, ServerMsg, SlotId, Target, TickMode};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
use bota_server::net::{DEFAULT_MESSAGES_PER_SEC, Listeners};
use bota_server::spectators::Spectators;
use proxy::{Dir, Line, Proxy};
use wire::{Wire, endpoint};

fn opts(mode: TickMode, tick_rate: u16) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode,
        tick_rate,
        players: 2,
        replay: None,
        seed: Some(5),
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
        dummy_seat: false,
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
        empty_after: None,
    }
}

/// Starts a server on free ports, the match on its own thread, and returns
/// the address that speaks `wire`.
fn serve(opts: ServerOpts, wire: Wire) -> SocketAddr {
    let (listeners, addr) = listen(wire);
    thread::spawn(move || run(listeners, opts));
    addr
}

fn listen(wire: Wire) -> (Listeners, SocketAddr) {
    let tcp = TcpListener::bind("127.0.0.1:0").expect("a port");
    let tcp_addr = tcp.local_addr().expect("bound");
    match wire {
        Wire::Tcp => (tcp.into(), tcp_addr),
        #[cfg(feature = "websocket")]
        Wire::Ws | Wire::Wss => {
            use bota_server::net::{Tls, WsListener};
            use std::path::Path;
            let ws = TcpListener::bind("127.0.0.1:0").expect("a port");
            let ws_addr = ws.local_addr().expect("bound");
            let tls = (wire == Wire::Wss).then(|| {
                Tls::from_pem_files(Path::new(wire::CERT), Path::new(wire::KEY))
                    .expect("the test TLS")
            });
            let ws = WsListener::new(ws, tls);
            (Listeners::with_websocket(tcp, ws), ws_addr)
        }
    }
}

over_every_wire!(
    a_match_over_slow_jittery_capped_and_stalling_lines_plays_to_its_normal_end,
    a_dropped_connection_comes_back_to_the_same_seat_and_hero_and_lockstep_waits_for_neither,
    a_stalled_client_slows_neither_the_tick_nor_the_other_player,
    a_half_open_connection_is_released_and_its_client_comes_back,
    a_threaded_net_plays_through_a_reset_and_comes_back_to_its_seat,
    a_clients_ping_and_order_receipts_are_the_lines_round_trip_and_not_a_multiple_of_the_tick,
);

/// The bot of the `attack_move` example.
fn attack_move(session: &ClientSession) -> Option<IssuedOrder> {
    let enemy = fountain_pos(session.map(), session.enemy());
    Some(IssuedOrder::hero(Order::Attack {
        target: Target::Pos(enemy),
    }))
}

/// Takes a seat at `address`, giving up on a silent server after `timeout`.
fn join_seat(address: Endpoint, name: &str, timeout: Duration) -> (Link, Seated) {
    Link::join_at(
        address,
        name,
        HeroId::SHADOW_FIEND,
        None,
        timeout,
        Redial::DEFAULT,
    )
    .expect("a seat")
}

/// Plays the attack-move bot to the end of the match on its own thread.
fn play(address: Endpoint, name: &'static str) -> thread::JoinHandle<Outcome> {
    thread::spawn(move || {
        Link::play_at(address, name, HeroId::SHADOW_FIEND, attack_move)
            .expect("the match is played to its end")
    })
}

fn a_match_over_slow_jittery_capped_and_stalling_lines_plays_to_its_normal_end(wire: Wire) {
    let server = serve(opts(TickMode::Realtime, 120), wire);
    let far = Proxy::start(server);
    let line = Line {
        latency: Duration::from_millis(150),
        jitter: Duration::from_millis(60),
        bytes_per_sec: Some(200_000),
    };
    far.set_line(Dir::Up, line);
    far.set_line(Dir::Down, line);
    let stalling = Proxy::start(server);
    stalling.set_line(
        Dir::Down,
        Line {
            latency: Duration::from_millis(50),
            ..Line::default()
        },
    );
    let a = play(endpoint(wire, &far.addr()), "far");
    let b = play(endpoint(wire, &stalling.addr()), "stalling");
    // A TCP stall of a lossy line: two seconds without a byte, now and then.
    while !b.is_finished() {
        thread::sleep(Duration::from_secs(3));
        stalling.stall(Dir::Down, true);
        thread::sleep(Duration::from_secs(2));
        stalling.stall(Dir::Down, false);
    }
    let (a, b) = (a.join().expect("far"), b.join().expect("stalling"));
    assert_eq!(a.winner, b.winner, "both saw the same end");
    assert_ne!(a.slot, b.slot);
    assert_eq!(
        (far.accepted(), stalling.accepted()),
        (1, 1),
        "no line broke"
    );
}

/// What a seat's bot saw of its own seat, tick by tick.
#[derive(Clone, Copy, Debug)]
struct Sight {
    tick: u32,
    slot: Option<SlotId>,
    hero: Option<HeroId>,
}

fn a_dropped_connection_comes_back_to_the_same_seat_and_hero_and_lockstep_waits_for_neither(
    wire: Wire,
) {
    let mut opts = opts(TickMode::Lockstep, 30);
    // Long enough that a seat waited for in vain would show as a stall.
    opts.ack_timeout_ticks = 900;
    let server = serve(opts, wire);
    let line = Proxy::start(server);
    let (cut_at, cut) = mpsc::channel();
    let (seen_tx, seen) = mpsc::channel();
    let address = endpoint(wire, &line.addr());
    let dropped = thread::spawn(move || {
        Link::play_at(address, "dropped", HeroId::SHADOW_FIEND, |session| {
            let tick = session.current().map_or(0, |view| view.tick);
            if tick == 1500 {
                let _ = cut_at.send(());
            }
            let _ = seen_tx.send(Sight {
                tick,
                slot: session.slot(),
                hero: session.own_player().map(|player| player.hero),
            });
            attack_move(session)
        })
        .expect("the match is played to its end")
    });
    let steady_address = endpoint(wire, &server.to_string());
    let steady = thread::spawn(move || {
        let (mut last, mut longest) = (None::<Instant>, Duration::ZERO);
        let outcome = Link::play_at(steady_address, "steady", HeroId::SHADOW_FIEND, |session| {
            longest = longest.max(last.map_or(Duration::ZERO, |last| last.elapsed()));
            last = Some(Instant::now());
            attack_move(session)
        })
        .expect("the match is played to its end");
        (outcome, longest)
    });
    cut.recv().expect("tick 1500 came");
    line.reset();
    let (dropped, (steady, longest)) = (
        dropped.join().expect("dropped"),
        steady.join().expect("steady"),
    );
    // A seat waited for in vain would hold a tick for the 30 s ack timeout.
    assert!(
        longest < Duration::from_secs(25),
        "neither the drop nor the return stalled the match: a tick waited {longest:?}"
    );
    assert_eq!(line.accepted(), 2, "the seat dialed again once");
    assert_eq!(dropped.winner, steady.winner);
    let sights: Vec<Sight> = seen.try_iter().collect();
    let first = sights.first().expect("ticks were seen");
    assert!(
        sights.iter().any(|sight| sight.tick > 1600),
        "the seat played on"
    );
    for sight in &sights {
        assert_eq!((sight.slot, sight.hero), (first.slot, first.hero));
    }
    assert!(sights.windows(2).all(|pair| pair[0].tick < pair[1].tick));
}

/// What a reading client shares with the test: its latest tick, and when to stop.
#[derive(Default)]
struct Reading {
    last_tick: AtomicU32,
    stop: AtomicBool,
}

/// Reads ticks until told to stop, returning when each snapshot came, its
/// tick and how many ticks it said were skipped.
fn read_ticks(link: &mut Link, reading: &Reading) -> Vec<(Instant, u32, usize)> {
    let mut ticks = Vec::new();
    while !reading.stop.load(Ordering::SeqCst) {
        if let Some(ServerMsg::Snapshot { view }) = link.recv().expect("the link holds") {
            ticks.push((Instant::now(), view.tick, view.skipped_ticks.len()));
            reading.last_tick.store(view.tick, Ordering::SeqCst);
        }
    }
    ticks
}

/// Waits until `done` holds, polling, or panics after `limit`.
fn wait_for(limit: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "{what} within {limit:?}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn a_stalled_client_slows_neither_the_tick_nor_the_other_player(wire: Wire) {
    let server = serve(opts(TickMode::Realtime, 30), wire);
    let line = Proxy::start(server);
    let join = |address: Endpoint, name: &'static str| {
        let reading = Arc::new(Reading::default());
        let shared = Arc::clone(&reading);
        let ticks = thread::spawn(move || {
            let (mut link, _) = join_seat(address, name, DEFAULT_IO_TIMEOUT);
            read_ticks(&mut link, &shared)
        });
        (reading, ticks)
    };
    let (stalled_reading, stalled) = join(endpoint(wire, &line.addr()), "stalled");
    let (steady_reading, steady) = join(endpoint(wire, &server.to_string()), "steady");
    let tick = |reading: &Reading| reading.last_tick.load(Ordering::SeqCst);
    wait_for(
        Duration::from_secs(30),
        "both clients see the match",
        || tick(&stalled_reading) > 30 && tick(&steady_reading) > 30,
    );
    line.stall(Dir::Down, true);
    let stall = Instant::now();
    thread::sleep(Duration::from_secs(3));
    line.stall(Dir::Down, false);
    wait_for(
        Duration::from_secs(30),
        "the stalled client catches up",
        || tick(&stalled_reading) + 10 >= tick(&steady_reading),
    );
    for reading in [&stalled_reading, &steady_reading] {
        reading.stop.store(true, Ordering::SeqCst);
    }
    let (stalled, steady) = (
        stalled.join().expect("stalled"),
        steady.join().expect("steady"),
    );
    let during = |ticks: &[(Instant, u32, usize)]| {
        let inside: Vec<u32> = ticks
            .iter()
            .filter(|(at, ..)| *at > stall + Duration::from_millis(500))
            .filter(|(at, ..)| *at < stall + Duration::from_millis(2_500))
            .map(|(_, tick, _)| *tick)
            .collect();
        (
            inside.len(),
            inside.first().copied(),
            inside.last().copied(),
        )
    };
    // Sixty ticks are due in those two seconds; a tick held up by the stall would show as few.
    let (count, first, last) = during(&steady);
    assert!(
        count >= 30,
        "the other player was sent {count} ticks in two seconds"
    );
    assert!(
        last.zip(first)
            .is_some_and(|(last, first)| last - first >= 30),
        "the clock kept its pace"
    );
    assert_eq!(during(&stalled).0, 0, "the stalled client was sent nothing");
    assert!(
        stalled.iter().any(|(.., skipped)| *skipped > 0),
        "the stalled client was skipped ahead, its ticks' events kept"
    );
}

fn a_half_open_connection_is_released_and_its_client_comes_back(wire: Wire) {
    let mut opts = opts(TickMode::Realtime, 30);
    let idle = Duration::from_secs(2);
    opts.idle_timeout = Some(idle);
    let server = serve(opts, wire);
    let line = Proxy::start(server);
    let address = endpoint(wire, &line.addr());
    let returning = thread::spawn(move || {
        let (mut link, seated) = join_seat(address, "returning", idle);
        let mut seat = Seat::new(seated, SessionOptions::default());
        let (mut resumed, mut after) = (None, 0);
        while after < 30 {
            let msg = link
                .recv()
                .expect("the link holds")
                .expect("the match goes on");
            match seat.ingest(msg, &mut ()).expect("a sound stream") {
                Ingested::Resumed { .. } => resumed = Some(Instant::now()),
                Ingested::TickComplete { .. } if resumed.is_some() => after += 1,
                _ => {}
            }
        }
        (seated.slot, resumed.expect("the session resumed"))
    });
    let (mut watcher, _) = join_seat(
        endpoint(wire, &server.to_string()),
        "watcher",
        DEFAULT_IO_TIMEOUT,
    );
    // The match runs, so both seats are held, before the line goes dark.
    while !matches!(
        watcher.recv().expect("the watcher's link holds"),
        Some(ServerMsg::Snapshot { .. })
    ) {}
    line.stall(Dir::Up, true);
    line.stall(Dir::Down, true);
    let holed = Instant::now();
    let left = loop {
        match watcher.recv().expect("the watcher's link holds") {
            Some(ServerMsg::ParticipantLeft { slot, .. }) => break slot,
            Some(_) => {}
            None => panic!("the server hung up on the watcher"),
        }
    };
    let released = holed.elapsed();
    let (slot, resumed) = returning.join().expect("returning");
    assert_eq!(left, Some(slot), "the half-open seat was let go");
    // Released by the idle timeout, not by anything quicker: the hole closes nothing.
    assert!(
        released + Duration::from_millis(1_500) >= idle,
        "released after {released:?}"
    );
    // The fresh connection is refused (SeatHeld) while the dead one holds the seat, and dials
    // again at most four seconds apart.
    let back = resumed.duration_since(holed);
    assert!(
        back < released + Duration::from_secs(60),
        "the client came back {back:?} after the hole, the seat freed after {released:?}"
    );
    assert_eq!(line.accepted(), 2);
}

fn a_threaded_net_plays_through_a_reset_and_comes_back_to_its_seat(wire: Wire) {
    let mut opts = opts(TickMode::Lockstep, 30);
    opts.ack_timeout_ticks = 900;
    let server = serve(opts, wire);
    let line = Proxy::start(server);
    let address = endpoint(wire, &line.addr());
    let (cut_at, cut) = mpsc::channel();
    let threaded = thread::spawn(move || {
        let mut net = Net::open(address, Redial::DEFAULT).expect("connect");
        let seated = join(&mut net, "threaded", HeroId::SHADOW_FIEND, None).expect("a seat");
        let mut decide = |session: &ClientSession| {
            if session.current().is_some_and(|view| view.tick == 1500) {
                let _ = cut_at.send(());
            }
            attack_move(session)
        };
        bota_client_core::play(&mut net, seated, &mut (), &mut decide)
            .expect("the match is played to its end")
    });
    let steady = play(endpoint(wire, &server.to_string()), "steady");
    cut.recv().expect("tick 1500 came");
    line.reset();
    let (threaded, steady) = (
        threaded.join().expect("threaded"),
        steady.join().expect("steady"),
    );
    assert!(line.accepted() >= 2, "the seat dialed again");
    assert_eq!(threaded.winner, steady.winner);
    assert!(threaded.ticks > 1600, "the seat played on");
}

/// What a threaded client measures in a running 30 Hz match over a line that adds `one_way` each
/// way: the smoothed ping, the smoothed time from sending an order to its receipt, and the
/// earliest tick the last order told it could take effect on.
fn measured_over(wire: Wire, one_way: Duration) -> (Duration, Duration, u32) {
    // One seat and a dummy: the match ticks at its own pace and the client is alone in it.
    let mut opts = opts(TickMode::Realtime, 30);
    opts.dummy_seat = true;
    let server = serve(opts, wire);
    let line = Proxy::start(server);
    let slow = Line {
        latency: one_way,
        ..Line::default()
    };
    line.set_line(Dir::Up, slow);
    line.set_line(Dir::Down, slow);
    let mut net = Net::open(endpoint(wire, &line.addr()), Redial::DEFAULT).expect("connect");
    join(&mut net, "timed", HeroId::SHADOW_FIEND, None).expect("a seat");
    // Polled at a frame's pace, as an app does; the probes go out and the echoes come back
    // between the ticks and the frames, and an order goes out twice a second.
    let started = Instant::now();
    let mut seq = 0;
    let mut next_order = started;
    while started.elapsed() < Duration::from_millis(4500) {
        net.poll();
        if Instant::now() >= next_order {
            seq += 1;
            next_order += Duration::from_millis(500);
            net.send(&ClientMsg::Order {
                seq,
                unit: None,
                order: Order::Move {
                    target: Target::None,
                },
                seen: None,
                queue: false,
            })
            .expect("an order goes out");
        }
        thread::sleep(Duration::from_millis(7));
    }
    let ping = net.ping().expect("an echo came back");
    let order = net.order_latency().expect("an order was received");
    let receipt = net.last_receipt().expect("an order was received");
    assert!(net.pending_orders() <= 1, "receipts come back");
    net.send(&ClientMsg::LeaveGame).expect("leave");
    (ping, order, receipt.tick)
}

fn a_clients_ping_and_order_receipts_are_the_lines_round_trip_and_not_a_multiple_of_the_tick(
    wire: Wire,
) {
    // Neither 50 ms nor 70 ms is a multiple of the 33.3 ms tick: a ping read off ticks would show
    // 33 or 66 for the first and 66 or 100 for the second, 16 ms or more off one of them. An
    // order is received when the server reads it, not on the tick that carries it out.
    let ms = Duration::from_millis;
    let (near, near_order, tick) = measured_over(wire, ms(25));
    let (far, far_order, _) = measured_over(wire, ms(35));
    assert!(
        near.abs_diff(ms(50)) < ms(10),
        "a 50 ms line measured {near:?}"
    );
    assert!(
        far.abs_diff(ms(70)) < ms(10),
        "a 70 ms line measured {far:?}"
    );
    assert!(
        far.saturating_sub(near).abs_diff(ms(20)) < ms(10),
        "20 ms of difference measured as {:?}",
        far.saturating_sub(near)
    );
    assert!(
        near_order.abs_diff(ms(50)) < ms(10),
        "an order over a 50 ms line was received after {near_order:?}"
    );
    assert!(
        far_order.abs_diff(ms(70)) < ms(10),
        "an order over a 70 ms line was received after {far_order:?}"
    );
    assert!(
        tick > 1,
        "the receipt names the tick the order can take effect on"
    );
}
