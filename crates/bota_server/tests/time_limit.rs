//! A match the service ends from outside, at its time limit or by asking, ends as a match: the
//! players are warned ahead of a limit, told who ended it, handed the numbers up to the end, and
//! the replay is closed whole and plays to that end. A connection that merely breaks is told
//! nothing: it is a network failure, not an ending.
//!
//! No test waits for time to pass: the limit counts on a clock the test moves (`ManualClock`), at
//! definite points of the stream (the first tick, the warning), and the match, which a seat that
//! stands still takes no further than a tick or two past them, never comes near the map's own
//! end.

#[allow(dead_code)] // the line simulator is shared; this test only resets a line
mod proxy;

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use bota_client_core::{
    Endpoint, Ingested, Link, Redial, Seat, Seated, SessionOptions, Transport, UnpackedReplay,
};
use bota_proto::{
    AbortReason, ClientMsg, FrameReader, HeroId, MapId, ReplayRecord, Role, ServerMsg, TickMode,
};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run_with};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::outside::{LimitClock, ManualClock, Outside, StopRequest, TimeLimit};
use bota_server::spectators::Spectators;

fn opts(replay: Option<std::path::PathBuf>) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 2,
        replay,
        replay_max_bytes: None,
        meta: None,
        meta_trailer: false,
        seed: Some(11),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(2),
        cheats: false,
        lobby_cheats: false,
        // Nothing the game itself does ends the match before the service does.
        immortal_structures: true,
        immortal_heroes: true,
        objectives: None,
        runes: None,
        dummy_seat: true,
        seated_start: false,
        ack_timeout_ticks: 150,
        // The machine is busy: nobody is let go for silence.
        idle_timeout: None,
        order_delays: Vec::new(),
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: Some(Duration::from_secs(30)),
    }
}

/// What a seat saw of the match until it ended.
#[derive(Debug, Default)]
struct Seen {
    /// The seconds the warning of the time limit gave.
    warned: Vec<u32>,
    /// How the stream ended: the reason and whether numbers came, or none.
    aborted: Option<(AbortReason, bool)>,
    /// Whether a match was played out ([`ServerMsg::MatchOver`]).
    over: bool,
    /// Ticks played.
    ticks: u32,
}

/// What a test does to the outside while its seat plays, at definite points of the stream.
#[derive(Default)]
struct Moves {
    /// The request to stop, set once the match is running (its first tick came), so that the
    /// service asks for the end of a match that has begun, whatever the machine's load.
    stop: Option<Arc<AtomicBool>>,
    /// The clock of the limit, with the time it is moved to when the first tick comes (the
    /// warning falls due) and the time it is moved to when the warning is told (the limit does).
    clock: Option<(ManualClock, Duration, Duration)>,
}

impl Moves {
    fn first_tick(&self) {
        if let Some(stop) = &self.stop {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some((clock, warning_due, _)) = &self.clock {
            clock.set(*warning_due);
        }
    }

    fn warned(&self) {
        if let Some((clock, _, limit_due)) = &self.clock {
            clock.set(*limit_due);
        }
    }
}

/// Plays a seat that stands still, acknowledging every tick, until the stream ends, and tells
/// what it saw, making `moves` as it goes. A stream that ends in a close with no ending is an
/// error of the test.
fn watch_to_the_end(mut link: Link, seated: Seated, moves: &Moves) -> Seen {
    let mut seat = Seat::new(seated, SessionOptions::default());
    let mut seen = Seen::default();
    loop {
        let msg = link
            .recv()
            .expect("the line holds")
            .expect("the stream ended with its own ending, not a bare close");
        match seat
            .ingest(msg, &mut ())
            .expect("a stream that makes sense")
        {
            Ingested::TickComplete { tick } => {
                if seen.ticks == 0 {
                    moves.first_tick();
                }
                seen.ticks = tick;
                let session = seat.session_mut().expect("a tick comes after the start");
                if let Some(tick) = session.end_turn() {
                    link.send(&ClientMsg::Ack { tick }).expect("an ack");
                }
            }
            Ingested::Passed(msg) => match *msg {
                ServerMsg::TimeLimitNear { remaining_secs } => {
                    seen.warned.push(remaining_secs);
                    moves.warned();
                }
                ServerMsg::Ping => link.send(&ClientMsg::Pong).expect("a pong"),
                _ => {}
            },
            Ingested::Aborted { reason, stats } => {
                seen.aborted = Some((reason, stats.is_some()));
                return seen;
            }
            Ingested::Over { .. } => {
                seen.over = true;
                return seen;
            }
            _ => {}
        }
    }
}

/// A one-seat match on a real socket, ended the way `outside` says; the seat's account of it, the
/// server's, and the replay's records.
fn play_ended_by(outside: Outside, moves: &Moves) -> (Seen, MatchEnd, Vec<ReplayRecord>) {
    let path = std::env::temp_dir().join(format!(
        "bota-time-limit-{}-{:p}.brp",
        std::process::id(),
        &outside
    ));
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let replay = path.clone();
    let server = thread::spawn(move || run_with(listener, opts(Some(replay)), outside));
    let (link, seated) =
        Link::join(&address, "standing", HeroId::SHADOW_FIEND, None).expect("a seat");
    let seen = watch_to_the_end(link, seated, moves);
    let end = server
        .join()
        .expect("the server thread")
        .expect("a clean end");
    let records = replay_records(&path);
    let _ = std::fs::remove_file(&path);
    (seen, end, records)
}

/// Every record of a replay file, which must unpack to its end and hold whole records.
fn replay_records(path: &std::path::Path) -> Vec<ReplayRecord> {
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut UnpackedReplay::new(std::fs::File::open(path).expect("the replay")),
        &mut bytes,
    )
    .expect("a stream closed whole unpacks to its end");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    let mut records = Vec::new();
    while let Some(record) = reader
        .next_message::<ReplayRecord>()
        .expect("a whole record")
    {
        records.push(record);
    }
    records
}

#[test]
fn a_match_at_its_time_limit_warns_ahead_then_ends_as_an_abort_with_its_replay_closed() {
    let limit = TimeLimit::new(Duration::from_secs(9), Duration::from_secs(4));
    let clock = ManualClock::new();
    let outside = Outside::on_clock(limit, StopRequest::NEVER, LimitClock::Manual(clock.clone()));
    let moves = Moves {
        stop: None,
        clock: Some((clock, Duration::from_secs(5), Duration::from_secs(9))),
    };
    let (seen, end, records) = play_ended_by(outside, &moves);
    assert_eq!(
        seen.warned,
        [4],
        "warned once, four seconds before the limit: {seen:?}"
    );
    assert_eq!(
        seen.aborted,
        Some((AbortReason::TimeLimit, true)),
        "told who ended it, with the numbers up to the end"
    );
    assert!(!seen.over, "no winner was announced");
    assert!(seen.ticks > 0, "the match was played: {seen:?}");
    let MatchEnd::Aborted {
        reason,
        report,
        replay_truncated,
    } = end
    else {
        panic!("the server reports an abort: {end:?}");
    };
    assert_eq!(reason, AbortReason::TimeLimit);
    assert!(!replay_truncated);
    let report = report.expect("the match as far as it went");
    assert!(report.stats.duration > 0);
    assert!(
        matches!(
            records.last(),
            Some(ReplayRecord::Msg(ServerMsg::MatchAborted {
                reason: AbortReason::TimeLimit,
                stats: Some(_)
            }))
        ),
        "the replay ends with the abort: {:?}",
        records.last()
    );
    assert!(
        !records
            .iter()
            .any(|record| matches!(record, ReplayRecord::Msg(ServerMsg::TimeLimitNear { .. }))),
        "a warning is no part of the match"
    );
}

#[test]
fn a_request_to_stop_ends_the_running_match_as_an_abort_by_the_service() {
    let flag = Arc::new(AtomicBool::new(false));
    let outside = Outside::new(None, StopRequest::on(Arc::clone(&flag)));
    let (seen, end, records) = play_ended_by(
        outside,
        &Moves {
            stop: Some(flag),
            clock: None,
        },
    );
    assert_eq!(seen.aborted, Some((AbortReason::Stopped, true)));
    assert!(seen.warned.is_empty(), "there was no limit to warn of");
    assert!(matches!(
        end,
        MatchEnd::Aborted {
            reason: AbortReason::Stopped,
            report: Some(_),
            ..
        }
    ));
    assert!(matches!(
        records.last(),
        Some(ReplayRecord::Msg(ServerMsg::MatchAborted {
            reason: AbortReason::Stopped,
            ..
        }))
    ));
}

#[test]
fn a_lobby_the_service_ends_before_the_match_is_told_so_without_numbers() {
    let flag = Arc::new(AtomicBool::new(false));
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let outside = Outside::new(None, StopRequest::on(Arc::clone(&flag)));
    let server = thread::spawn(move || run_with(listener, opts(None), outside));
    let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
    bota_client_core::greet(&mut link, Role::Player, "waiting", None).expect("a seat");
    flag.store(true, Ordering::Relaxed);
    let told = loop {
        match link.recv().expect("the line holds") {
            Some(ServerMsg::MatchAborted { reason, stats }) => break (reason, stats),
            Some(_) => {}
            None => panic!("the lobby closed without a word"),
        }
    };
    assert_eq!(told, (AbortReason::Stopped, None));
    let end = server
        .join()
        .expect("the server thread")
        .expect("a clean end");
    assert!(matches!(
        end,
        MatchEnd::Aborted {
            reason: AbortReason::Stopped,
            report: None,
            ..
        }
    ));
}

#[test]
fn a_connection_that_breaks_is_told_no_ending_and_its_seat_waits() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let flag = Arc::new(AtomicBool::new(false));
    let outside = Outside::new(None, StopRequest::on(Arc::clone(&flag)));
    let server = thread::spawn(move || run_with(listener, opts(None), outside));
    let (mut link, seated) =
        Link::join(&address, "dropping", HeroId::SHADOW_FIEND, None).expect("a seat");
    let mut seat = Seat::new(seated, SessionOptions::default());
    // Into the match, then the line breaks with nothing said.
    loop {
        let msg = link.recv().expect("the line holds").expect("a message");
        if matches!(
            seat.ingest(msg, &mut ()).expect("sensible"),
            Ingested::Started { .. }
        ) {
            break;
        }
    }
    drop(link);
    // A broken line ends nothing: the match goes on until the service asks, and then ends as the
    // service's own end, not as anything the broken line caused (an end of the match decided by
    // the line, before the request, would be that end here).
    flag.store(true, Ordering::Relaxed);
    let end = server
        .join()
        .expect("the server thread")
        .expect("a clean end");
    assert!(matches!(
        end,
        MatchEnd::Aborted {
            reason: AbortReason::Stopped,
            ..
        }
    ));
}

/// Starts a server whose only seat takes the first tick and then never acknowledges it, with an
/// acknowledgement timeout (30 s) far longer than any wait of these tests, and gives the seat's
/// line once the server holds the tick for it.
fn server_held_by_a_seat_that_does_not_acknowledge(outside: Outside) -> Link {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let mut options = opts(None);
    options.ack_timeout_ticks = 900;
    thread::spawn(move || run_with(listener, options, outside));
    let (mut link, seated) = Link::join(&address, "stalling", HeroId::SHADOW_FIEND, None).unwrap();
    let mut seat = Seat::new(seated, SessionOptions::default());
    loop {
        let msg = link.recv().unwrap().unwrap();
        if matches!(
            seat.ingest(msg, &mut ()).unwrap(),
            Ingested::TickComplete { .. }
        ) {
            break;
        }
    }
    // A probe answered proves the server has the tick out and now waits for the missing ack.
    link.send(&ClientMsg::Probe { id: 1 }).unwrap();
    while !matches!(link.recv().unwrap(), Some(ServerMsg::Echo { id: 1 })) {}
    link
}

/// The reason the stream ends with, within `within`, or none.
fn abort_reason_within(mut link: Link, within: Duration) -> Option<AbortReason> {
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        while let Ok(Some(msg)) = link.recv() {
            if let ServerMsg::MatchAborted { reason, .. } = msg {
                let _ = tx.send(reason);
                return;
            }
        }
    });
    rx.recv_timeout(within).ok()
}

#[test]
fn a_request_to_stop_ends_a_lockstep_match_held_by_a_seat_that_does_not_acknowledge() {
    let flag = Arc::new(AtomicBool::new(false));
    let outside = Outside::new(None, StopRequest::on(Arc::clone(&flag)));
    let link = server_held_by_a_seat_that_does_not_acknowledge(outside);
    flag.store(true, Ordering::Relaxed);
    assert_eq!(
        abort_reason_within(link, Duration::from_secs(10)),
        Some(AbortReason::Stopped),
        "the stop does not wait for the 30 s acknowledgement timeout"
    );
}

#[test]
fn a_time_limit_ends_a_lockstep_match_held_by_a_seat_that_does_not_acknowledge() {
    let limit = TimeLimit::new(Duration::from_secs(4), Duration::from_secs(1));
    let clock = ManualClock::new();
    let outside = Outside::on_clock(limit, StopRequest::NEVER, LimitClock::Manual(clock.clone()));
    let link = server_held_by_a_seat_that_does_not_acknowledge(outside);
    clock.set(Duration::from_secs(4));
    assert_eq!(
        abort_reason_within(link, Duration::from_secs(15)),
        Some(AbortReason::TimeLimit),
        "the limit does not wait for the 30 s acknowledgement timeout"
    );
}

/// The messages of a line until it ends with the abort: the seconds of every warning, in order.
fn warnings_until_abort(link: &mut Link) -> Vec<u32> {
    let mut warned = Vec::new();
    loop {
        match link.recv().expect("the line holds") {
            Some(ServerMsg::TimeLimitNear { remaining_secs }) => warned.push(remaining_secs),
            Some(ServerMsg::MatchAborted { .. }) => return warned,
            Some(_) => {}
            None => panic!("the lobby closed without a word"),
        }
    }
}

#[test]
fn a_seat_that_joins_after_the_warning_is_told_what_is_left_once() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let limit = TimeLimit::new(Duration::from_secs(8), Duration::from_secs(5));
    let clock = ManualClock::new();
    let outside = Outside::on_clock(limit, StopRequest::NEVER, LimitClock::Manual(clock.clone()));
    let mut options = opts(None);
    options.dummy_seat = false;
    let server = thread::spawn(move || run_with(listener, options, outside));
    let mut early = Link::connect(&address, Duration::from_secs(60)).expect("connected");
    bota_client_core::greet(&mut early, Role::Player, "early", None).expect("a seat");
    // The warning (at most half the limit ahead of it: four seconds) falls due four seconds in;
    // the lobby is still waiting for its second seat.
    clock.set(Duration::from_secs(4));
    while !matches!(
        early.recv().expect("the line holds"),
        Some(ServerMsg::TimeLimitNear { remaining_secs: 4 })
    ) {}
    let mut late = Link::connect(&address, Duration::from_secs(60)).expect("connected");
    bota_client_core::greet(&mut late, Role::Player, "late", None).expect("a seat");
    clock.set(Duration::from_secs(8));
    let warned = warnings_until_abort(&mut late);
    assert_eq!(
        warned,
        [4],
        "told once, and not twice, the seconds that are left"
    );
    server.join().expect("the server thread").expect("an end");
}

/// A seat that heard the match ended does not dial again, whatever its line does after: a
/// connection that breaks is dialed again, an ending is no break. Over a real line that is
/// reset the moment after the ending, with the client trying to talk on.
#[test]
fn a_seat_that_heard_the_match_aborted_does_not_dial_again_when_its_line_is_reset() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_address = listener.local_addr().unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let outside = Outside::new(None, StopRequest::on(Arc::clone(&flag)));
    thread::spawn(move || run_with(listener, opts(None), outside));
    let line = proxy::Proxy::start(server_address);
    let endpoint = Endpoint::parse(&line.addr()).expect("an endpoint");
    let (mut link, _seated) = Link::join_at(
        endpoint,
        "ended",
        HeroId::SHADOW_FIEND,
        None,
        Duration::from_secs(60),
        Redial::DEFAULT,
    )
    .expect("a seat");
    flag.store(true, Ordering::Relaxed);
    loop {
        if matches!(
            link.recv().expect("the line holds"),
            Some(ServerMsg::MatchAborted { .. })
        ) {
            break;
        }
    }
    line.reset();
    // The client goes on talking (it acknowledges what it saw) into a line that is gone.
    for _ in 0..5 {
        let _ = link.send(&ClientMsg::Ack { tick: 1 });
        let _ = link.recv();
    }
    assert_eq!(line.accepted(), 1, "no second connection was made");
}
