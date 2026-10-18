//! The server and the client library over bad lines: high latency and
//! jitter, a bandwidth cap, stalls, resets and a black hole, made by an
//! in-process proxy between them.

mod proxy;

use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use bota_client_core::{
    ClientSession, Ingested, IssuedOrder, Link, Outcome, Seat, SessionOptions, Transport,
};
use bota_core::fountain_pos;
use bota_proto::{HeroId, MapId, Order, ServerMsg, SlotId, Target, TickMode};
use bota_server::game_loop::{DEFAULT_IDLE_TIMEOUT, ServerOpts, run};
use proxy::{Dir, Line, Proxy};

fn opts(mode: TickMode, tick_rate: u16) -> ServerOpts {
    ServerOpts {
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
        dummy_seat: false,
        ack_timeout_ticks: 150,
        idle_timeout: Some(DEFAULT_IDLE_TIMEOUT),
        order_delays: Vec::new(),
    }
}

/// Starts a server on a free port; the match runs on its own thread.
fn serve(opts: ServerOpts) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let addr = listener.local_addr().expect("bound");
    thread::spawn(move || run(listener, opts));
    addr
}

/// The bot of the `attack_move` example.
fn attack_move(session: &ClientSession) -> Option<IssuedOrder> {
    let enemy = fountain_pos(session.map(), session.enemy());
    Some(IssuedOrder::hero(Order::Attack {
        target: Target::Pos(enemy),
    }))
}

/// Plays the attack-move bot to the end of the match on its own thread.
fn play(address: String, name: &'static str) -> thread::JoinHandle<Outcome> {
    thread::spawn(move || {
        Link::play(&address, name, HeroId::SHADOW_FIEND, attack_move)
            .expect("the match is played to its end")
    })
}

#[test]
fn a_match_over_slow_jittery_capped_and_stalling_lines_plays_to_its_normal_end() {
    let server = serve(opts(TickMode::Realtime, 120));
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
    let a = play(far.addr(), "far");
    let b = play(stalling.addr(), "stalling");
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

#[test]
fn a_dropped_connection_comes_back_to_the_same_seat_and_hero_and_lockstep_waits_for_neither() {
    let mut opts = opts(TickMode::Lockstep, 30);
    // Long enough that a seat waited for in vain would show as a stall.
    opts.ack_timeout_ticks = 900;
    let server = serve(opts);
    let line = Proxy::start(server);
    let (cut_at, cut) = mpsc::channel();
    let (seen_tx, seen) = mpsc::channel();
    let address = line.addr();
    let dropped = thread::spawn(move || {
        Link::play(&address, "dropped", HeroId::SHADOW_FIEND, |session| {
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
    let steady = thread::spawn(move || {
        let (mut last, mut longest) = (None::<Instant>, Duration::ZERO);
        let outcome = Link::play(
            &server.to_string(),
            "steady",
            HeroId::SHADOW_FIEND,
            |session| {
                longest = longest.max(last.map_or(Duration::ZERO, |last| last.elapsed()));
                last = Some(Instant::now());
                attack_move(session)
            },
        )
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
        longest < Duration::from_secs(10),
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

#[test]
fn a_stalled_client_slows_neither_the_tick_nor_the_other_player() {
    let server = serve(opts(TickMode::Realtime, 30));
    let line = Proxy::start(server);
    let join = |address: String, name: &'static str| {
        let reading = Arc::new(Reading::default());
        let shared = Arc::clone(&reading);
        let ticks = thread::spawn(move || {
            let (mut link, _) =
                Link::join(&address, name, HeroId::SHADOW_FIEND, None).expect("a seat");
            read_ticks(&mut link, &shared)
        });
        (reading, ticks)
    };
    let (stalled_reading, stalled) = join(line.addr(), "stalled");
    let (steady_reading, steady) = join(server.to_string(), "steady");
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

#[test]
fn a_half_open_connection_is_released_and_its_client_comes_back() {
    let mut opts = opts(TickMode::Realtime, 30);
    let idle = Duration::from_secs(2);
    opts.idle_timeout = Some(idle);
    let server = serve(opts);
    let line = Proxy::start(server);
    let address = line.addr();
    let returning = thread::spawn(move || {
        let (mut link, seated) =
            Link::join_with_timeout(&address, "returning", HeroId::SHADOW_FIEND, None, idle)
                .expect("a seat");
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
    let (mut watcher, _) =
        Link::join(&server.to_string(), "watcher", HeroId::SHADOW_FIEND, None).expect("a seat");
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
        back < released + Duration::from_secs(10),
        "the client came back {back:?} after the hole, the seat freed after {released:?}"
    );
    assert_eq!(line.accepted(), 2);
}
