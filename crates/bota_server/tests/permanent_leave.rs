//! Permanent departures remain final and survive a capped replay.

use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use bota_client_core::{Link, Transport, join, play};
use bota_proto::{ClientMsg, HeroId, MapId, SeatKey, ServerMsg, Team, TickMode};
use bota_replay::meta::{MetaRecord, decode};
use bota_server::game::OrderDelay;
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run};
use bota_server::lobby::ReservedSeat;
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

fn seat(key: u8, name: &str, team: Team) -> ReservedSeat {
    ReservedSeat {
        key: SeatKey([key; 32]),
        name: name.to_owned(),
        team,
        kind: None,
    }
}

fn opts(reserved: Vec<ReservedSeat>) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 3,
        replay: None,
        replay_max_bytes: None,
        meta: None,
        meta_trailer: false,
        seed: Some(5),
        match_id: None,
        reserved,
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(2),
        cheats: false,
        lobby_cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: false,
        seated_start: false,
        ack_timeout_ticks: 150,
        idle_timeout: None,
        order_delays: vec![OrderDelay { base: 1, jitter: 0 }; 3],
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: Some(Duration::from_secs(5)),
    }
}

/// Acknowledges every tick; once `leave_at` is acknowledged it sends LeaveGame and hangs up.
struct LeaveAndHangUp {
    link: Link,
    leave_at: u32,
}

impl Transport for LeaveAndHangUp {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.link.recv()
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        self.link.send(msg)?;
        match msg {
            ClientMsg::Ack { tick } if *tick >= self.leave_at => {
                self.link.send(&ClientMsg::LeaveGame)?;
                Err(std::io::Error::other("hung up after leaving"))
            }
            _ => Ok(()),
        }
    }
}

/// Acknowledges every tick; once `leave_at` is acknowledged it sends LeaveGame and plays on.
struct LeaveAndStay {
    link: Link,
    leave_at: u32,
    sent: bool,
}

impl Transport for LeaveAndStay {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.link.recv()
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        self.link.send(msg)?;
        if let ClientMsg::Ack { tick } = msg
            && *tick >= self.leave_at
            && !self.sent
        {
            self.sent = true;
            self.link.send(&ClientMsg::LeaveGame)?;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// The key was refused after the holder left for good.
    Refused,
    /// The key took the seat again after the holder left for good.
    Reseated,
    /// The seat stayed held for the whole wait: nothing was learned.
    Inconclusive,
}

/// The host leaves at tick 60, hangs up, and then asks for its seat back with its key.
fn host_leaves_then_asks_again(address: &str) -> Verdict {
    let mut link = Link::connect(address, Duration::from_secs(60)).expect("connected");
    let seated = join(
        &mut link,
        "host",
        HeroId::SHADOW_FIEND,
        Some(SeatKey([1; 32])),
    )
    .expect("a seat");
    let mut leaving = LeaveAndHangUp { link, leave_at: 60 };
    let _ = play(&mut leaving, seated, &mut (), |_| Vec::new());
    drop(leaving);
    let started = Instant::now();
    loop {
        let mut again = Link::connect(address, Duration::from_secs(60)).expect("connected");
        match join(
            &mut again,
            "host",
            HeroId::SHADOW_FIEND,
            Some(SeatKey([1; 32])),
        ) {
            Ok(_) => {
                drop(again);
                return Verdict::Reseated;
            }
            Err(error) if error.to_string().contains("SeatHeld") => {}
            Err(_) => return Verdict::Refused,
        }
        if started.elapsed() > Duration::from_secs(20) {
            return Verdict::Inconclusive;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_seat_whose_holder_left_for_good_is_not_taken_back_with_its_key() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let reserved = vec![
        seat(1, "host", Team::Radiant),
        seat(2, "friend", Team::Radiant),
        seat(3, "enemy", Team::Dire),
    ];
    let server = thread::spawn(move || run(listener, opts(reserved)));
    let host = {
        let address = address.clone();
        thread::spawn(move || host_leaves_then_asks_again(&address))
    };
    let friend = {
        let address = address.clone();
        thread::spawn(move || {
            let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
            let seated = join(
                &mut link,
                "friend",
                HeroId::SHADOW_FIEND,
                Some(SeatKey([2; 32])),
            )
            .expect("a seat");
            play(&mut link, seated, &mut (), |_| Vec::new()).expect("played");
        })
    };
    let enemy = {
        let address = address.clone();
        thread::spawn(move || {
            let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
            let seated = join(
                &mut link,
                "enemy",
                HeroId::SHADOW_FIEND,
                Some(SeatKey([3; 32])),
            )
            .expect("a seat");
            let mut leaving = LeaveAndHangUp {
                link,
                leave_at: 240,
            };
            let _ = play(&mut leaving, seated, &mut (), |_| Vec::new());
        })
    };
    let verdict = host.join().expect("the host thread");
    friend.join().expect("the friend thread");
    enemy.join().expect("the enemy thread");
    let end = server.join().expect("the server").expect("it ran");
    let MatchEnd::Over { report, .. } = end else {
        panic!("the match ends with a result: {end:?}");
    };
    assert_eq!(
        verdict,
        Verdict::Refused,
        "the host sent LeaveGame at tick 60 and its departure is recorded at {:?}; \
         its key still took the seat back",
        report.stats.leaves
    );
}

/// A cap the first few hundred ticks of this match reach well before its 900th tick (the whole
/// 900-tick replay compresses to about 40 KiB).
const CAP: u64 = 16 << 10;

#[test]
fn a_departure_after_the_replay_cap_is_in_the_metadata_the_result_reports() {
    for (trailer, cap, has_trailer) in [
        (false, CAP, false),
        (true, CAP, false),
        (true, CAP * 2, true),
    ] {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("permanent_leave_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch directory");
        let replay = dir.join(format!("cap-{}-{trailer}-{cap}.brp", std::process::id()));
        let meta = replay.with_extension("meta");
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = listener.local_addr().expect("bound").to_string();
        let options = ServerOpts {
            replay: Some(replay.clone()),
            replay_max_bytes: Some(cap),
            meta_trailer: trailer,
            meta: Some(meta.clone()),
            players: 2,
            dummy_seat: true,
            order_delays: Vec::new(),
            empty_after: None,
            map: MapId(2),
            seed: Some(3),
            ..opts(Vec::new())
        };
        let server = thread::spawn(move || run(listener, options));
        let (link, seated) =
            Link::join(&address, "leaver", HeroId::SHADOW_FIEND, None).expect("a seat");
        let mut leaving = LeaveAndStay {
            link,
            leave_at: 900,
            sent: false,
        };
        let _ = play(&mut leaving, seated, &mut (), |_| Vec::new());
        let end = server.join().expect("the server").expect("it ran");
        let MatchEnd::Over {
            report,
            replay_truncated,
            ..
        } = end
        else {
            panic!("the match ends with a result: {end:?}");
        };
        assert!(
            replay_truncated,
            "precondition: the replay was closed at its cap"
        );
        let reported: Vec<(u8, u32)> = report
            .stats
            .leaves
            .iter()
            .map(|left| (left.slot.0, left.tick))
            .collect();
        assert!(
            !reported.is_empty(),
            "precondition: the result reports the departure"
        );
        let recorded = decode(&std::fs::read(&meta).expect("metadata")).expect("metadata decodes");
        let leaves: Vec<(u8, u32)> = recorded
            .records
            .iter()
            .filter_map(|record| match record {
                MetaRecord::Leave { slot, tick } => Some((*slot, *tick)),
                _ => None,
            })
            .collect();
        assert_eq!(
            leaves, reported,
            "the metadata of a replay closed at its cap lost the departure the result reports"
        );
        let prefix = bota_replay::trailer::find(&mut std::fs::File::open(&replay).expect("replay"))
            .expect("trailer read");
        assert_eq!(
            prefix.is_some(),
            has_trailer,
            "a trailer needs room within its whole-file cap"
        );
        if let Some(prefix) = prefix {
            let prefix = decode(&prefix).expect("prefix metadata");
            assert!(
                !prefix.complete,
                "a capped prefix carries no later match result"
            );
            assert!(
                !prefix
                    .records
                    .iter()
                    .any(|record| matches!(record, MetaRecord::Leave { .. })),
                "the prefix trailer must not describe the later departure"
            );
        }
    }
}

#[test]
fn public_profile_provenance_survives_full_and_capped_replays_without_seat_secrets() {
    use bota_replay::meta::Profiles;
    for (label, context) in [
        ("local", Profiles::default()),
        (
            "public",
            Profiles::new(
                Some("wss://match.example/control".into()),
                vec![(0, Some(42)), (1, None)],
            )
            .expect("public context"),
        ),
    ] {
        for cap in [None, Some(CAP * 2)] {
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("profile-provenance-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("scratch");
            let replay = dir.join(format!("{label}-{cap:?}.brp"));
            let meta = replay.with_extension("meta");
            let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
            let address = listener.local_addr().expect("address").to_string();
            let options = ServerOpts {
                replay: Some(replay.clone()),
                meta: Some(meta.clone()),
                meta_trailer: true,
                replay_max_bytes: cap,
                players: 2,
                dummy_seat: true,
                order_delays: Vec::new(),
                empty_after: None,
                ..opts(Vec::new())
            };
            let launch = context.clone();
            let server = thread::spawn(move || {
                bota_server::game_loop::run_with_profiles(listener, options, launch)
            });
            let (link, seated) =
                Link::join(&address, "player", HeroId::SHADOW_FIEND, None).expect("seat");
            let mut leaving = LeaveAndStay {
                link,
                leave_at: 900,
                sent: false,
            };
            let _ = play(&mut leaving, seated, &mut (), |_| Vec::new());
            let end = server
                .join()
                .expect("server thread")
                .expect("server result");
            assert!(matches!(end, MatchEnd::Over { .. }));
            let full = decode(&std::fs::read(&meta).expect("sidecar")).expect("metadata");
            assert!(full.complete);
            assert_eq!(
                full.profiles(),
                Some(context.clone()),
                "sidecar identifies the matchmaker and public accounts"
            );
            let carried =
                bota_replay::trailer::find(&mut std::fs::File::open(&replay).expect("replay"))
                    .expect("trailer read")
                    .expect("trailer");
            let prefix = decode(&carried).expect("prefix metadata");
            assert_eq!(prefix.profiles(), Some(context.clone()));
            assert_eq!(
                prefix.complete,
                cap.is_none(),
                "a capped trailer describes only its recorded prefix"
            );
            if let Some(cap) = cap {
                assert!(std::fs::metadata(&replay).expect("size").len() <= cap);
            }
            std::fs::remove_file(replay).expect("remove replay");
            std::fs::remove_file(meta).expect("remove metadata");
        }
    }
}

#[test]
fn a_mismatched_public_profile_roster_is_rejected_before_ready() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bota_server"))
        .args([
            "--port",
            "0",
            "--players",
            "2",
            "--matchmaker",
            "wss://match.example/control",
            "--profile-accounts",
            "42",
        ])
        .env_remove("BOTA_SEATS")
        .env_remove("BOTA_REFEREE_KEY")
        .output()
        .expect("server exits after invalid launch context");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("profile accounts must match the roster in slot order")
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line == "READY"),
        "invalid provenance must not advertise a usable server"
    );
}
