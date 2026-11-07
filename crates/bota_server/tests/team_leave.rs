//! A seat that leaves the match for good while a teammate still plays: the match runs on for the
//! others, as Dota's does, and ends only as matches end (a side gone, or an empty match after
//! `empty_after`). The teammate here dropped its first connection and took its seat back with its
//! key: what the leaver last saw of the lobby says nothing of who plays on, so the server, not
//! the leaver's client, decides when the match ends.

use std::net::TcpListener;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

use bota_client_core::{Link, Transport, join, play};
use bota_proto::{ClientMsg, HeroId, MapId, SeatKey, ServerMsg, Team, TickMode};
use bota_server::game::OrderDelay;
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run};
use bota_server::lobby::ReservedSeat;
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

/// How long a test waits for what must come, at most: a hang guard, not a measure.
const HANG_GUARD: Duration = Duration::from_secs(300);

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
        // A client waits on another or on a slow line, and the machine is busy: nobody is let go
        // for silence unless the test is about that.
        idle_timeout: None,
        order_delays: vec![OrderDelay { base: 1, jitter: 0 }; 3],
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        // An empty match ends: the server of a game whose last player left stops by itself.
        empty_after: Some(Duration::from_secs(5)),
    }
}

/// A link that leaves the match for good once it has acknowledged `leave_at`, and `go` has said
/// to: a seat that is still empty when its teammate's leave is heard does not count as held by
/// anyone, which is a different scenario from the one under test.
struct Leaving {
    link: Link,
    leave_at: Option<u32>,
    go: Option<Receiver<()>>,
}

impl Transport for Leaving {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.link.recv()
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        self.link.send(msg)?;
        if let (ClientMsg::Ack { tick }, Some(at)) = (msg, self.leave_at)
            && *tick >= at
        {
            self.leave_at = None;
            if let Some(go) = self.go.take() {
                go.recv_timeout(HANG_GUARD)
                    .expect("the teammate's seat never came back");
            }
            self.link.send(&ClientMsg::LeaveGame)?;
            self.link.send(&ClientMsg::LeaveGame)?;
        }
        Ok(())
    }
}

fn seat(key: u8, name: &str, team: Team) -> ReservedSeat {
    ReservedSeat {
        key: SeatKey([key; 32]),
        name: name.to_owned(),
        team,
        kind: None,
    }
}

/// Joins `address` with `key` as `name`, leaving for good at `leave_at`, and plays to the end:
/// the team that won, and the last tick shown.
fn player(
    address: String,
    key: u8,
    name: &'static str,
    leave_at: Option<u32>,
    go: Option<Receiver<()>>,
) -> thread::JoinHandle<(Team, u32)> {
    thread::spawn(move || {
        let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
        let seated = join(
            &mut link,
            name,
            HeroId::SHADOW_FIEND,
            Some(SeatKey([key; 32])),
        )
        .expect("a seat");
        let mut leaving = Leaving { link, leave_at, go };
        let outcome = play(&mut leaving, seated, &mut (), |_| Vec::new()).expect("played");
        (outcome.winner, outcome.ticks)
    })
}

/// The friend: drops its connection at tick 30, then takes its seat back with its key, tells
/// `back` and plays to the end.
fn friend_dropping_then_back(address: &str, back: &Sender<()>) -> (Team, u32) {
    let key = Some(SeatKey([2; 32]));
    let mut first = Link::connect(address, Duration::from_secs(60)).expect("connected");
    let seated = join(&mut first, "friend", HeroId::SHADOW_FIEND, key).expect("a seat");
    let mut dropping = Dropping {
        link: first,
        drop_at: 30,
    };
    // Ends when the link drops itself: the connection goes with it.
    let _ = play(&mut dropping, seated, &mut (), |_| Vec::new());
    drop(dropping);
    let started = std::time::Instant::now();
    loop {
        assert!(
            started.elapsed() < HANG_GUARD,
            "the friend's seat never came back"
        );
        let mut link = Link::connect(address, Duration::from_secs(60)).expect("connected");
        if let Ok(seated) = join(&mut link, "friend", HeroId::SHADOW_FIEND, key) {
            back.send(()).expect("the host waits for the friend");
            let outcome = play(&mut link, seated, &mut (), |_| Vec::new()).expect("played");
            return (outcome.winner, outcome.ticks);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn recorded_leaves(path: &std::path::Path) -> Vec<bota_proto::MatchLeave> {
    use std::io::Read;
    let mut stream =
        bota_client_core::UnpackedReplay::new(std::fs::File::open(path).expect("replay"));
    let mut reader = bota_proto::FrameReader::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut last_snapshot = 0;
    let mut leaves = Vec::new();
    loop {
        let count = stream.read(&mut buffer).expect("replay stream");
        if count == 0 {
            break;
        }
        reader.push(&buffer[..count]);
        while let Some(record) = reader
            .next_message::<bota_proto::ReplayRecord>()
            .expect("frame")
        {
            match record {
                bota_proto::ReplayRecord::Msg(ServerMsg::Snapshot { view }) => {
                    last_snapshot = view.tick
                }
                bota_proto::ReplayRecord::Leave { tick, slot } => {
                    assert_eq!(
                        tick, last_snapshot,
                        "a departure names its acceptance boundary before the next advance"
                    );
                    leaves.push(bota_proto::MatchLeave { slot, tick });
                }
                _ => {}
            }
        }
    }
    leaves
}

#[test]
fn a_match_runs_on_for_a_teammate_after_a_seat_leaves_for_good() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let reserved = vec![
        seat(1, "host", Team::Radiant),
        seat(2, "friend", Team::Radiant),
        seat(3, "enemy", Team::Dire),
    ];
    let directory =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/recording_tests");
    std::fs::create_dir_all(&directory).expect("recording directory");
    let meta = directory.join(format!("team-leave-{}.meta", std::process::id()));
    let replay = meta.with_extension("brp");
    let mut options = opts(reserved);
    options.meta = Some(meta.clone());
    options.replay = Some(replay.clone());
    options.meta_trailer = true;
    let server = thread::spawn(move || run(listener, options));
    // The friend's connection drops once the match is on, and its seat is taken back with the key
    // before the host leaves: a drop in the lobby, or a leave heard while the seat is empty, are
    // other scenarios.
    let (back, host_may_leave) = channel();
    let host = player(address.clone(), 1, "host", Some(60), Some(host_may_leave));
    let enemy = player(address.clone(), 3, "enemy", Some(240), None);
    let friend = {
        let address = address.clone();
        thread::spawn(move || friend_dropping_then_back(&address, &back))
    };
    host.join().expect("the host played");
    let (friend_won, friend_ticks) = friend.join().expect("the friend played");
    let (enemy_won, _) = enemy.join().expect("the enemy played");
    assert!(
        friend_ticks > 240,
        "the match ran on after the host left: the friend saw tick {friend_ticks}"
    );
    assert_eq!((friend_won, enemy_won), (Team::Radiant, Team::Radiant));
    let end = server.join().expect("the server").expect("it ran");
    assert!(
        matches!(
            end,
            MatchEnd::Over {
                winner: Team::Radiant,
                left: Some(_),
                ..
            }
        ),
        "the side that stayed won when the other left: {end:?}"
    );
    let MatchEnd::Over { report, .. } = end else {
        unreachable!()
    };
    assert_eq!(
        report
            .stats
            .leaves
            .iter()
            .map(|left| left.slot.0)
            .collect::<Vec<_>>(),
        [0, 2],
        "both permanent departures, not the dropped/reclaimed teammate"
    );
    assert!(report.stats.leaves[0].tick >= 60);
    assert!(report.stats.leaves[0].tick < report.stats.leaves[1].tick);
    assert!(report.stats.leaves[1].tick >= 240);
    assert!(report.stats.leaves[1].tick < report.stats.duration);
    assert_eq!(recorded_leaves(&replay), report.stats.leaves);
    let bytes = std::fs::read(&meta).expect("metadata");
    let rebuilt = bota_client_core::meta_of_replay(std::fs::File::open(&replay).expect("replay"))
        .expect("offline metadata");
    assert_eq!(rebuilt, bytes, "live and offline recording agree");
    let recorded = bota_replay::meta::decode(&bytes).expect("read metadata");
    let leaves: Vec<_> = recorded
        .records
        .iter()
        .filter_map(|record| match record {
            bota_replay::meta::MetaRecord::Leave { tick, slot } => Some((*slot, *tick)),
            _ => None,
        })
        .collect();
    assert_eq!(
        leaves,
        report
            .stats
            .leaves
            .iter()
            .map(|left| (left.slot.0, left.tick))
            .collect::<Vec<_>>()
    );
}

/// A link that drops its connection, without leaving, once it has acknowledged `drop_at`.
struct Dropping {
    link: Link,
    drop_at: u32,
}

impl Transport for Dropping {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.link.recv()
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        self.link.send(msg)?;
        match msg {
            ClientMsg::Ack { tick } if *tick >= self.drop_at => {
                Err(std::io::Error::other("dropped"))
            }
            _ => Ok(()),
        }
    }
}

/// Every seat drops its connection mid-match, nobody leaving for good: the seats may be taken
/// back, so the match waits, and once every seat has been empty for `empty_after` it ends with
/// no result (the server of a client-hosted game that everyone left ends by itself).
#[test]
fn a_match_everyone_dropped_ends_once_it_was_empty_long_enough() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let mut opts = opts(vec![
        seat(1, "host", Team::Radiant),
        seat(2, "friend", Team::Radiant),
        seat(3, "enemy", Team::Dire),
    ]);
    opts.empty_after = Some(Duration::from_secs(1));
    let directory =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/recording_tests");
    std::fs::create_dir_all(&directory).expect("recording directory");
    let meta = directory.join(format!("team-drop-{}.meta", std::process::id()));
    opts.meta = Some(meta.clone());
    let (ended, end) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = ended.send(run(listener, opts));
    });
    let players: Vec<_> = [(1, "host"), (2, "friend"), (3, "enemy")]
        .into_iter()
        .map(|(key, name)| {
            let address = address.clone();
            thread::spawn(move || {
                let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
                let seated = join(
                    &mut link,
                    name,
                    HeroId::SHADOW_FIEND,
                    Some(SeatKey([key; 32])),
                )
                .expect("a seat");
                let mut dropping = Dropping { link, drop_at: 30 };
                // Ends when the link drops itself: the connection goes with it.
                let _ = play(&mut dropping, seated, &mut (), |_| Vec::new());
            })
        })
        .collect();
    for player in players {
        player.join().expect("a player");
    }
    let end = end
        .recv_timeout(HANG_GUARD)
        .expect("the empty match ended")
        .expect("it ran");
    assert_eq!(
        end,
        MatchEnd::Abandoned { leaves: Vec::new() },
        "no result for a match nobody plays"
    );
    let recorded =
        bota_replay::meta::decode(&std::fs::read(meta).expect("metadata")).expect("read metadata");
    assert!(!recorded.complete);
    assert!(
        !recorded
            .records
            .iter()
            .any(|record| matches!(record, bota_replay::meta::MetaRecord::Leave { .. })),
        "ordinary disconnections remain reconnectable even when the server abandons an empty match"
    );
}
