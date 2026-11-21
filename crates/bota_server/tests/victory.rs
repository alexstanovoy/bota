//! The end of a match a fallen Ancient decides. Dota's 1v1 ends as any game does, the loser's
//! Ancient destroyed, on the very tick its tower or its second death decides it; the service is
//! told the result at once; and the players are kept, the world standing as it fell, until each
//! has left (its client showing it the end), at most for the linger, and then let go cleanly.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bota_client_core::{ClientSession, IssuedOrder, Link, Transport, UnpackedReplay};
use bota_proto::{
    Cheat, ClientMsg, EventKind, FrameReader, HeroId, MapId, Order, ReplayRecord, ServerMsg,
    Target, Team, TickMode, UnitKind,
};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run_with};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::outside::{LimitClock, ManualClock, Outside, StopRequest};
use bota_server::spectators::Spectators;

/// How long a test waits for the server to speak or end. It guards against a hang only.
const WAIT: Duration = Duration::from_secs(300);

/// What a test waits for a connection to answer or end: real time, generous for a loaded machine.
const LINK_WAIT: Duration = Duration::from_secs(30);

/// A linger no test lets run out by itself: its clock is moved by hand.
const LINGER: Duration = Duration::from_secs(60);

fn opts(replay: Option<PathBuf>) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 1,
        replay,
        replay_max_bytes: None,
        meta: None,
        meta_trailer: false,
        seed: Some(18),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(2),
        cheats: true,
        lobby_cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: false,
        seated_start: false,
        ack_timeout_ticks: 150,
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

/// How the lone Radiant player ends its solo mid match.
#[derive(Clone, Copy, Debug)]
enum Decider {
    /// The Dire tower it sees first falls to a cheat: Radiant wins.
    Tower,
    /// Its own hero dies twice: Dire wins.
    TwoDeaths,
}

/// The orders of a lone Radiant player that end the match by `decider`, one decision a tick.
fn deciding(decider: Decider) -> impl FnMut(&ClientSession) -> Vec<IssuedOrder> {
    let mut decisions = 0u32;
    move |session| {
        decisions += 1;
        let cheat = |cheat| IssuedOrder::hero(Order::Cheat { cheat });
        match (decider, decisions) {
            (_, 1) => vec![cheat(Cheat::AllVision { on: true })],
            (_, 2) => vec![cheat(Cheat::StartGame)],
            (Decider::TwoDeaths, 3 | 6) => vec![cheat(Cheat::Suicide)],
            (Decider::TwoDeaths, 5) => vec![cheat(Cheat::Respawn)],
            (Decider::Tower, 3..) => session
                .current()
                .and_then(|view| {
                    view.units
                        .iter()
                        .find(|unit| unit.kind == UnitKind::Tower && unit.team == Team::Dire)
                })
                .map(|tower| {
                    cheat(Cheat::Destroy {
                        target: Target::Unit(tower.id),
                    })
                })
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// The records of a finished replay file, header first.
fn records(path: &Path) -> Vec<ReplayRecord> {
    let file = std::fs::File::open(path).expect("the replay exists");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut UnpackedReplay::new(file), &mut bytes)
        .expect("a finished stream");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    std::iter::from_fn(|| reader.next_message::<ReplayRecord>().expect("a record")).collect()
}

/// Every building the replay tells of falling: its tick, kind and side, in the replay's order.
fn falls(records: &[ReplayRecord]) -> Vec<(u32, UnitKind, Team)> {
    records
        .iter()
        .filter_map(|record| match record {
            ReplayRecord::Msg(ServerMsg::Events { tick, events }) => Some((*tick, events)),
            _ => None,
        })
        .flat_map(|(tick, events)| {
            events.iter().filter_map(move |event| match event {
                EventKind::StructureDestroyed { kind, team, .. } => Some((tick, *kind, *team)),
                _ => None,
            })
        })
        .collect()
}

/// Plays a solo mid match to its end by `decider` with a replay, and returns the replay's
/// records and the end.
fn solo_mid_ended_by(decider: Decider) -> (Vec<ReplayRecord>, MatchEnd) {
    let replay = std::env::temp_dir().join(format!(
        "bota-victory-{}-{decider:?}.brp",
        std::process::id()
    ));
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let options = opts(Some(replay.clone()));
    let server = thread::spawn(move || run_with(listener, options, Outside::default()));
    let outcome = Link::play(&address, "decider", HeroId::SHADOW_FIEND, deciding(decider))
        .expect("the match is played to its end");
    let end = ended(server).expect("it ran");
    let winner = match decider {
        Decider::Tower => Team::Radiant,
        Decider::TwoDeaths => Team::Dire,
    };
    assert_eq!(outcome.winner, winner, "{outcome:?}");
    let records = records(&replay);
    let _ = std::fs::remove_file(&replay);
    (records, end)
}

/// The deciding fall and the loser's Ancient's on one tick, the Ancient's last, and nothing but
/// the end after it.
fn assert_the_losers_ancient_fell_with_the_decision(records: &[ReplayRecord], loser: Team) {
    let falls = falls(records);
    let ancients: Vec<_> = falls
        .iter()
        .filter(|(_, kind, _)| *kind == UnitKind::Ancient)
        .collect();
    assert_eq!(ancients.len(), 1, "one Ancient falls: {falls:?}");
    let (tick, _, team) = *ancients[0];
    assert_eq!(team, loser, "the loser's Ancient: {falls:?}");
    assert_eq!(
        falls.last(),
        Some(ancients[0]),
        "the Ancient falls last: {falls:?}"
    );
    let over = records
        .iter()
        .position(|record| matches!(record, ReplayRecord::Msg(ServerMsg::MatchOver { .. })))
        .expect("the replay tells the end");
    let ended_at = match &records[over] {
        ReplayRecord::Msg(ServerMsg::MatchOver { stats, .. }) => stats.duration,
        _ => unreachable!("found as the end"),
    };
    assert_eq!(tick, ended_at, "the Ancient falls on the deciding tick");
    assert_eq!(
        over,
        records.len() - 1,
        "the end is the replay's last record"
    );
}

#[test]
fn a_solo_mid_match_decided_by_a_tower_destroys_the_losers_ancient_on_the_deciding_tick() {
    let (records, end) = solo_mid_ended_by(Decider::Tower);
    assert!(
        matches!(
            end,
            MatchEnd::Over {
                winner: Team::Radiant,
                left: None,
                ..
            }
        ),
        "{end:?}"
    );
    let falls = falls(&records);
    assert!(
        falls
            .iter()
            .any(|(_, kind, team)| *kind == UnitKind::Tower && *team == Team::Dire),
        "the tower decided it: {falls:?}"
    );
    assert_the_losers_ancient_fell_with_the_decision(&records, Team::Dire);
}

#[test]
fn a_solo_mid_match_decided_by_a_second_death_destroys_the_losers_ancient_on_the_deciding_tick() {
    let (records, end) = solo_mid_ended_by(Decider::TwoDeaths);
    assert!(
        matches!(
            end,
            MatchEnd::Over {
                winner: Team::Dire,
                left: None,
                ..
            }
        ),
        "{end:?}"
    );
    assert_the_losers_ancient_fell_with_the_decision(&records, Team::Radiant);
}

/// A server that lingers on a clock the test moves and tells its end to the test.
struct Lingering {
    server: thread::JoinHandle<std::io::Result<MatchEnd>>,
    told: mpsc::Receiver<MatchEnd>,
    clock: ManualClock,
    link: Link,
}

/// A solo mid match played to its end by a tower, its player still connected.
fn decided_and_lingering() -> Lingering {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let clock = ManualClock::new();
    let (tell, told) = mpsc::channel();
    let outside = Outside::on_clock(None, StopRequest::NEVER, LimitClock::Manual(clock.clone()))
        .lingering(LINGER)
        .telling(tell);
    let server = thread::spawn(move || run_with(listener, opts(None), outside));
    let (mut link, seated) =
        Link::join(&address, "lingering", HeroId::SHADOW_FIEND, None).expect("a seat");
    let outcome = bota_client_core::play(&mut link, seated, &mut (), deciding(Decider::Tower))
        .expect("the match is played to its end");
    assert_eq!(outcome.winner, Team::Radiant, "{outcome:?}");
    Lingering {
        server,
        told,
        clock,
        link,
    }
}

/// The server thread's result, once it has ended: waited for a bounded time, so a server that does not end fails
/// the test with what it was waited for instead of hanging it.
fn ended<T>(server: thread::JoinHandle<T>) -> T {
    let deadline = Instant::now() + LINK_WAIT;
    while !server.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the server did not end within {LINK_WAIT:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
    server.join().expect("the server ends")
}

/// Whether the connection still answers: its probe is echoed. The server's side answers it on
/// the connection's own thread, so only an open connection does.
fn answers(link: &mut Link) -> bool {
    const PROBE: u32 = 0x5eed;
    if link.send(&ClientMsg::Probe { id: PROBE }).is_err() {
        return false;
    }
    let deadline = Instant::now() + LINK_WAIT;
    while Instant::now() < deadline {
        match link.recv() {
            Ok(Some(ServerMsg::Echo { id })) if id == PROBE => return true,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return false,
        }
    }
    false
}

#[test]
fn the_service_is_told_the_result_at_the_decision_while_the_player_is_still_kept() {
    let Lingering {
        server,
        told,
        clock: _clock,
        mut link,
    } = decided_and_lingering();
    let end = told.recv_timeout(WAIT).expect("the end is told");
    assert!(
        matches!(
            end,
            MatchEnd::Over {
                winner: Team::Radiant,
                ..
            }
        ),
        "{end:?}"
    );
    assert!(
        answers(&mut link),
        "the player is kept after the result was told"
    );
    assert!(!server.is_finished(), "the server lingers");
    drop(link);
    let returned = ended(server).expect("it ran");
    assert_eq!(
        returned, end,
        "the last player leaving ends the linger, the end as told"
    );
}

#[test]
fn a_player_who_never_leaves_is_let_go_cleanly_at_the_linger_bound() {
    let Lingering {
        server,
        told,
        clock,
        mut link,
    } = decided_and_lingering();
    told.recv_timeout(WAIT).expect("the end is told");
    assert!(answers(&mut link), "kept within the linger");
    clock.set(LINGER);
    // The server hangs up: the stream ends cleanly, nothing is reset.
    let deadline = Instant::now() + LINK_WAIT;
    loop {
        assert!(
            Instant::now() < deadline,
            "the server did not end the stream within {LINK_WAIT:?} of the clock reaching the linger bound"
        );
        match link.recv() {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(error) => panic!(
                "waiting for the server to hang up at the linger bound, the connection broke or timed out instead of ending: {error}"
            ),
        }
    }
    ended(server).expect("it ran");
}

#[test]
fn a_request_to_stop_ends_the_linger() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let outside = Outside::on_clock(
        None,
        StopRequest::on(std::sync::Arc::clone(&stop)),
        LimitClock::Manual(ManualClock::new()),
    )
    .lingering(LINGER);
    let server = thread::spawn(move || run_with(listener, opts(None), outside));
    let (mut link, seated) =
        Link::join(&address, "stopped", HeroId::SHADOW_FIEND, None).expect("a seat");
    bota_client_core::play(&mut link, seated, &mut (), deciding(Decider::Tower))
        .expect("the match is played to its end");
    assert!(answers(&mut link), "kept within the linger");
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let end = ended(server).expect("it ran");
    assert!(
        matches!(
            end,
            MatchEnd::Over {
                winner: Team::Radiant,
                ..
            }
        ),
        "a stop after the end changes nothing of it: {end:?}"
    );
}

/// The server binary as a launcher runs it, with one unreserved seat and cheats: its stdout read
/// line by line on a thread of its own.
struct Binary {
    child: std::process::Child,
    lines: mpsc::Receiver<String>,
}

impl Drop for Binary {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Binary {
    fn start(linger_secs: &str) -> (Binary, String) {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_bota_server"))
            .args(["--bind", "127.0.0.1", "--port", "0", "--map", "2"])
            .args(["--players", "1", "--cheats", "--mode", "lockstep"])
            .args(["--end-linger-secs", linger_secs, "--result-version", "3"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the server binary starts");
        let stdout = child.stdout.take().expect("piped");
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let binary = Binary { child, lines };
        let listening = binary.line();
        let address = listening
            .strip_prefix("bota_server listening on ")
            .expect("the port is announced first")
            .to_owned();
        assert_eq!(binary.line(), "READY");
        (binary, address)
    }

    fn line(&self) -> String {
        self.lines.recv_timeout(WAIT).expect("a line on stdout")
    }
}

#[test]
fn the_binary_prints_its_result_at_the_decision_and_exits_0_once_its_player_has_left() {
    let (mut binary, address) = Binary::start("120");
    let (mut link, seated) =
        Link::join(&address, "launched", HeroId::SHADOW_FIEND, None).expect("a seat");
    bota_client_core::play(&mut link, seated, &mut (), deciding(Decider::Tower))
        .expect("the match is played to its end");
    let result = binary.line();
    assert!(result.starts_with("RESULT winner=0 "), "{result}");
    assert!(
        result.ends_with(" artifacts=closed"),
        "the files are closed before the line, for the service to act on it at once: {result}"
    );
    assert!(
        answers(&mut link),
        "the player is kept after the result was printed"
    );
    drop(link);
    let deadline = std::time::Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = binary.child.try_wait().expect("waited on") {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the server exits once its player left"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{status:?}");
}
