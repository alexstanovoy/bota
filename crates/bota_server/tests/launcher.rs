//! The server binary as a launcher runs it: `READY` once players may
//! connect, one `RESULT` line and exit 0 at the end of a match, exit 3 for a
//! match nobody played, seats reserved by `key_hex:team:name`.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bota_client_core::{ClientSession, IssuedOrder, Link, Outcome, PlayError, UnpackedReplay};
use bota_core::fountain_pos;
use bota_proto::{
    AbortReason, FrameReader, HeroId, MatchReport, Order, ReplayRecord, SeatKey, ServerMsg, SlotId,
    Target, Team, decode_match_report,
};

/// How long a test waits for the server binary to speak or exit. It guards against a hang
/// only: starting a process and playing a match take as long as a busy machine makes them.
const WAIT: Duration = Duration::from_secs(300);
const KEYS: [SeatKey; 2] = [SeatKey([1; 32]), SeatKey([2; 32])];

/// The running server binary, killed when the test ends.
struct Server {
    child: Child,
    addr: String,
    stdout: mpsc::Receiver<String>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    /// The next line on stdout.
    fn line(&self, limit: Duration) -> String {
        self.stdout.recv_timeout(limit).expect("a line on stdout")
    }

    /// Waits for the exit, within `limit`.
    fn exit(&mut self, limit: Duration) -> ExitStatus {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().expect("waited on") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the server exits within {limit:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

fn lines_of(pipe: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    rx
}

/// Starts the binary on a loopback port with both seats reserved, slot 0 on
/// Dire and slot 1 on Radiant, and waits for `READY`.
fn start(args: &[&str]) -> Server {
    start_limited(args, None)
}

/// As [`start`], under a file size limit of `fsize_kib` KiB (what `docker --ulimit fsize` sets),
/// when given.
fn start_limited(args: &[&str], fsize_kib: Option<u32>) -> Server {
    start_with(args, fsize_kib, &[])
}

/// As [`start`], with these environment variables (what a launcher sets) besides the seats'.
fn start_env(args: &[&str], env: &[(&str, &str)]) -> Server {
    start_with(args, None, env)
}

fn start_with(args: &[&str], fsize_kib: Option<u32>, env: &[(&str, &str)]) -> Server {
    let seats = format!("{}:1:first,{}:0:second", KEYS[0].to_hex(), KEYS[1].to_hex());
    let mut command = match fsize_kib {
        None => Command::new(env!("CARGO_BIN_EXE_bota_server")),
        Some(kib) => {
            let mut command = Command::new("bash");
            command
                .args(["-c", "ulimit -f \"$0\" && exec \"$@\""])
                .arg(kib.to_string())
                .arg(env!("CARGO_BIN_EXE_bota_server"));
            command
        }
    };
    let mut child = command
        .args(["--bind", "127.0.0.1", "--port", "0", "--map", "2"])
        .args(args)
        .env("BOTA_SEATS", seats)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the server binary starts");
    let stdout = lines_of(child.stdout.take().expect("piped"));
    let mut server = Server {
        child,
        addr: String::new(),
        stdout,
    };
    let listening = server.line(WAIT);
    server.addr = listening
        .strip_prefix("bota_server listening on ")
        .expect("the port is announced first")
        .to_owned();
    assert!(server.addr.starts_with("127.0.0.1:"), "{listening}");
    assert_eq!(server.line(WAIT), "READY");
    server
}

fn attack_move(session: &ClientSession) -> Option<IssuedOrder> {
    let enemy = fountain_pos(session.map(), session.enemy());
    Some(IssuedOrder::hero(Order::Attack {
        target: Target::Pos(enemy),
    }))
}

fn play(addr: String, key: SeatKey, hero: HeroId) -> thread::JoinHandle<Outcome> {
    thread::spawn(move || {
        let (mut link, seated) = Link::join(&addr, "bot", hero, Some(key)).expect("a seat");
        bota_client_core::play(&mut link, seated, &mut (), attack_move)
            .expect("the match is played to its end")
    })
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "{hex}");
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex"))
        .collect()
}

#[test]
fn a_match_played_to_its_end_says_its_result_on_one_line_and_exits_0() {
    let mut server = start(&["--mode", "lockstep", "--ack-timeout-ticks", "900"]);
    let first = play(server.addr.clone(), KEYS[0], HeroId::PUDGE);
    let second = play(server.addr.clone(), KEYS[1], HeroId::SHADOW_FIEND);
    let (first, second) = (first.join().expect("first"), second.join().expect("second"));
    assert_eq!((first.slot, first.team), (SlotId(0), Team::Dire));
    assert_eq!((second.slot, second.team), (SlotId(1), Team::Radiant));

    let result = server.line(WAIT);
    let fields: Vec<&str> = result
        .strip_prefix("RESULT ")
        .expect("a result line")
        .split(' ')
        .collect();
    let [told_winner, duration, report] = fields[..] else {
        panic!("three fields: {result}");
    };
    let winner = match first.winner {
        Team::Radiant => "winner=0",
        Team::Dire => "winner=1",
        Team::Neutral => "winner=draw",
    };
    assert_eq!(told_winner, winner);
    let report = hex_bytes(report.strip_prefix("report=").expect("the report"));
    assert!(
        report.starts_with(b"BMRP\x01"),
        "RESULT report needs a versioned header"
    );
    let report: MatchReport = decode_match_report(&report).expect("a match report");
    let heroes: Vec<(SlotId, Team, HeroId)> = report
        .picks
        .iter()
        .map(|pick| (pick.slot, pick.team, pick.hero))
        .collect();
    assert_eq!(
        heroes,
        [
            (SlotId(0), Team::Dire, HeroId::PUDGE),
            (SlotId(1), Team::Radiant, HeroId::SHADOW_FIEND)
        ]
    );
    // Game time at 30 ticks a second.
    let ms = u64::from(report.stats.duration) * 1000 / 30;
    assert_eq!(duration, format!("duration_ms={ms}"));
    assert!(server.exit(WAIT).success());
}

/// The frames of a replay, unpacked to the end of its stream: a stream cut off mid-way fails.
fn replay_records(path: &std::path::Path) -> Vec<ReplayRecord> {
    let mut bytes = Vec::new();
    UnpackedReplay::new(std::fs::File::open(path).expect("the replay"))
        .read_to_end(&mut bytes)
        .expect("the replay's stream is closed whole");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    let mut records = Vec::new();
    while let Some(record) = reader.next_message().expect("a whole record") {
        records.push(record);
    }
    records
}

/// The long match behind the docker `--ulimit fsize` bug: a replay growing past the process's file
/// size limit. The server keeps under the limit, closes the replay whole and plays the match to
/// its end. A launcher that reads result line version 2 is told the replay was cut short; one
/// that does not (any older one) gets the line of the first servers, exactly.
#[test]
fn a_replay_under_a_file_size_limit_is_closed_whole_and_the_match_goes_on() {
    for version in ["1", "2"] {
        truncated_replay_under_limit(version);
    }
}

fn truncated_replay_under_limit(version: &str) {
    let path = std::env::temp_dir().join(format!("bota-fsize-{}.brp", std::process::id()));
    let replay = path.to_str().expect("a path");
    let limit_kib = 64;
    let mut server = start_limited(
        &[
            "--mode",
            "lockstep",
            "--ack-timeout-ticks",
            "900",
            "--replay",
            replay,
            "--result-version",
            version,
        ],
        Some(limit_kib),
    );
    let first = play(server.addr.clone(), KEYS[0], HeroId::PUDGE);
    let second = play(server.addr.clone(), KEYS[1], HeroId::SHADOW_FIEND);
    first.join().expect("the first seat plays to the end");
    second.join().expect("the second seat plays to the end");

    let result = server.line(WAIT);
    assert!(result.starts_with("RESULT winner="), "{result}");
    if version == "1" {
        assert_eq!(
            result.split(' ').count(),
            4,
            "the first line's fields only: {result}"
        );
        assert!(!result.contains("replay="), "{result}");
    } else {
        assert!(
            result.ends_with(" replay=truncated"),
            "the result says the replay was cut short: {result}"
        );
    }
    assert!(server.exit(WAIT).success());
    let size = std::fs::metadata(&path).expect("the replay").len();
    assert!(size <= u64::from(limit_kib) << 10, "{size} bytes");
    let records = replay_records(&path);
    let _ = std::fs::remove_file(&path);
    assert!(matches!(records[0], ReplayRecord::Header(_)));
    assert!(
        !records
            .iter()
            .any(|record| matches!(record, ReplayRecord::Msg(ServerMsg::MatchOver { .. }))),
        "the replay ends before the match did"
    );
}

/// A write past the limit that the server could not foresee must not end the process either:
/// SIGXFSZ is survived (its default action kills).
#[test]
fn a_file_size_signal_does_not_end_the_server() {
    let mut server = start(&["--mode", "lockstep", "--ack-timeout-ticks", "900"]);
    let sent = Command::new("kill")
        .args(["-s", "XFSZ", &server.child.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(sent.success());
    let first = play(server.addr.clone(), KEYS[0], HeroId::PUDGE);
    let second = play(server.addr.clone(), KEYS[1], HeroId::SHADOW_FIEND);
    first.join().expect("the first seat plays to the end");
    second.join().expect("the second seat plays to the end");
    assert!(server.line(WAIT).starts_with("RESULT winner="));
    assert!(server.exit(WAIT).success());
}

/// The seats of a match the service ends: they play on until the server tells them it is over,
/// whatever it says; what each was told comes back, and `started` hears of the first tick.
fn play_until_told(
    addr: String,
    key: SeatKey,
    hero: HeroId,
) -> (thread::JoinHandle<Option<AbortReason>>, mpsc::Receiver<()>) {
    let (started, ticking) = mpsc::channel();
    let seat = thread::spawn(move || {
        let (mut link, seated) = Link::join(&addr, "bot", hero, Some(key)).expect("a seat");
        // Standing still: the match is not over before the service ends it.
        let mut told = false;
        match bota_client_core::play(&mut link, seated, &mut (), |_: &ClientSession| {
            if !std::mem::replace(&mut told, true) {
                let _ = started.send(());
            }
            None::<IssuedOrder>
        }) {
            Err(PlayError::Aborted(reason)) => Some(reason),
            Ok(_) => None,
            Err(error) => panic!("the stream broke instead of ending: {error}"),
        }
    });
    (seat, ticking)
}

/// The time limit a launcher gives (`RADOVID_MAX_DURATION_SECS`) is read by the server, counted on
/// its wall clock from its start (a second here, the least there is), and ends a match nobody
/// came to as an abort for the time limit: the line says so, the exit code is the one of a match
/// nobody played, and no result is said. What a match that began says at its limit (the result
/// line, the code for a launcher that cannot read it) is `aborted_tests` of the binary; what its
/// players are told and what its replay holds is `time_limit.rs`, on a clock they move by hand.
// This is the one test of the unit that waits for real time, and it cannot be otherwise: the limit
// is a wall-clock limit of another process, and 0 is no limit (the flag refuses it, and a limit
// that counted 0 as expired would end every match of a misconfigured service at once), so the
// least limit there is, one second, is waited out. Its outcome cannot depend on the machine's
// load: nobody joins and nothing else can end or delay the match (no seat to wait for, no
// tick to spend, no cap to reach), so the limit is the only thing left to happen.
#[test]
fn the_limit_a_launcher_gave_is_read_and_ends_a_match_nobody_came_to_as_a_time_limit() {
    let mut server = start_env(&[], &[("RADOVID_MAX_DURATION_SECS", "1")]);
    assert_eq!(server.line(WAIT), "ABORTED reason=time_limit");
    assert_eq!(server.exit(WAIT).code(), Some(3));
    assert_eq!(
        server.stdout.recv_timeout(WAIT),
        Err(mpsc::RecvTimeoutError::Disconnected),
        "no result is said for a match nobody played"
    );
}

/// SIGTERM is how a launcher asks a game to finish: the match ends by the service's request.
#[test]
fn a_launcher_that_asks_with_sigterm_gets_the_match_ended_as_stopped() {
    let mut server = start_env(
        &[
            // Paced by the clock, so that a match of seats that stand still cannot reach the
            // map's own end (a draw at its tick cap) between the first tick and the signal.
            "--mode",
            "realtime",
            "--immortal-structures",
            "--immortal-heroes",
        ],
        &[("RADOVID_RESULT_VERSION", "3")],
    );
    let (first, first_ticking) = play_until_told(server.addr.clone(), KEYS[0], HeroId::PUDGE);
    let (second, second_ticking) =
        play_until_told(server.addr.clone(), KEYS[1], HeroId::SHADOW_FIEND);
    // The request comes once the match is running, whatever the machine's load.
    first_ticking.recv().expect("the first seat's first tick");
    second_ticking.recv().expect("the second seat's first tick");
    let sent = Command::new("kill")
        .args(["-s", "TERM", &server.child.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(sent.success());
    assert_eq!(first.join().expect("first"), Some(AbortReason::Stopped));
    assert_eq!(second.join().expect("second"), Some(AbortReason::Stopped));
    assert_eq!(server.line(WAIT), "ABORTED reason=stopped");
    let result = server.line(WAIT);
    assert!(result.contains(" aborted=stopped v=3"), "{result}");
    assert!(server.exit(WAIT).success());
}

#[test]
fn a_match_nobody_came_to_is_abandoned_with_exit_code_3() {
    let mut server = start(&["--start-after-secs", "1", "--abandon-unjoined"]);
    assert_eq!(server.exit(WAIT).code(), Some(3));
    assert_eq!(
        server.stdout.recv_timeout(WAIT),
        Err(mpsc::RecvTimeoutError::Disconnected),
        "no result is said for a match nobody played"
    );
}

#[test]
fn a_match_everyone_left_is_abandoned_once_it_stood_empty_long_enough() {
    let mut server = start(&["--start-after-secs", "1", "--empty-after-secs", "1"]);
    // One seat comes, waits for the start (the other seat's hero is drawn),
    // then leaves for good.
    let (mut link, _) =
        Link::join(&server.addr, "bot", HeroId::PUDGE, Some(KEYS[0])).expect("a seat");
    while !matches!(link_recv(&mut link), ServerMsg::Snapshot { .. }) {}
    let left = Instant::now();
    drop(link);
    assert_eq!(server.exit(WAIT).code(), Some(3));
    assert!(
        left.elapsed() >= Duration::from_secs(1),
        "{:?}",
        left.elapsed()
    );
    assert_eq!(
        server.stdout.recv_timeout(WAIT),
        Err(mpsc::RecvTimeoutError::Disconnected),
        "no result"
    );
}

fn link_recv(link: &mut Link) -> ServerMsg {
    use bota_client_core::Transport;
    link.recv()
        .expect("the link holds")
        .expect("the server speaks")
}

/// Economic events are trusted local setup (`MatchConfig::economy_events`, for in-process
/// matches only): nothing a launcher or a matchmaking service sets, no argument and no
/// environment variable, can name one, so a match of reserved seats never carries one.
#[test]
fn a_launched_match_cannot_carry_economic_events() {
    let help = Command::new(env!("CARGO_BIN_EXE_bota_server"))
        .arg("--help")
        .output()
        .expect("the server binary runs");
    assert!(help.status.success());
    let options = String::from_utf8_lossy(&help.stdout).to_lowercase();
    assert!(options.contains("--map"), "the help lists the options");
    assert!(
        !options.contains("econom"),
        "no option or environment variable names an economic event"
    );
    let seats = format!("{}:1:first,{}:0:second", KEYS[0].to_hex(), KEYS[1].to_hex());
    let refused = Command::new(env!("CARGO_BIN_EXE_bota_server"))
        .args(["--bind", "127.0.0.1", "--port", "0", "--map", "2"])
        .args(["--economy-events", "0:60:1000:0"])
        .env("BOTA_SEATS", seats)
        .stdin(Stdio::null())
        .output()
        .expect("the server binary runs");
    assert!(!refused.status.success(), "an unknown option is refused");
    assert!(
        !String::from_utf8_lossy(&refused.stdout).contains("READY"),
        "and no match is offered"
    );
}

#[test]
fn a_pick_phase_departure_is_reported_before_the_abandoned_server_exits() {
    use bota_client_core::{Transport, greet};
    use bota_proto::{ClientMsg, Role};
    let mut server = start(&["--mode", "lockstep"]);
    let mut link = Link::connect(&server.addr, WAIT).expect("connected");
    greet(&mut link, Role::Player, "first", Some(KEYS[0])).expect("reserved seat");
    link.send(&ClientMsg::LeaveGame)
        .expect("permanent departure");
    assert_eq!(server.exit(WAIT).code(), Some(3));
    let output: Vec<_> = server.stdout.iter().collect();
    assert!(
        output.iter().any(|line| line == "ABANDONED leaves=0@0"),
        "a pick-phase leaver must be reported, not silently cancelled: {output:?}"
    );
}
