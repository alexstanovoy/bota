//! Chat end to end: who hears a line, what the replay keeps of it, what is
//! refused, and that the match is the same match with it or without it; and
//! a compressed link, which never carries chat compressed.

use std::path::PathBuf;

use bota_proto::{
    Audience, Body, Codec, EntityId, Heard, Line, Ping, PingAt, PingKind, ReplayRecord, SayRefusal,
    SeatKey, Speaker,
};

use super::spectators::drain;
use super::*;
use crate::chat::{ChatOpts, SeatChat};
use crate::lobby::ReservedSeat;
use crate::spectators::Spectators;

const KEYS: [SeatKey; 2] = [SeatKey([31; 32]), SeatKey([32; 32])];

/// A team line in three scripts and an emoji, an all line, the lobby's.
const TEAM_TEXT: &str = "мид мой, 下路 来 한국어 👍";
const ALL_TEXT: &str = "gg wp — ありがとう";
const LOBBY_TEXT: &str = "glhf ✌";
const SPECTATOR_TEXT: &str = "смотрю";

fn say(to: Audience, text: &str) -> ClientMsg {
    ClientMsg::Say(Line {
        to,
        body: Body::Text(text.to_owned()),
    })
}

/// The lines among `messages`, by audience and text.
fn heard(messages: &[ServerMsg]) -> Vec<Heard> {
    messages
        .iter()
        .filter_map(|msg| match msg {
            ServerMsg::Heard(heard) => Some(heard.clone()),
            _ => None,
        })
        .collect()
}

fn texts(lines: &[Heard]) -> Vec<&str> {
    lines
        .iter()
        .map(|heard| match &heard.line.body {
            Body::Text(text) => text.as_str(),
            Body::Ping(_) => "(ping)",
        })
        .collect()
}

fn refusals(messages: &[ServerMsg]) -> Vec<SayRefusal> {
    messages
        .iter()
        .filter_map(|msg| match msg {
            ServerMsg::SayRefused { reason } => Some(*reason),
            _ => None,
        })
        .collect()
}

/// Two reserved seats, open spectators, a replay, and an end soon after
/// everyone leaves.
fn chat_match(seed: u64, replay: PathBuf) -> ServerOpts {
    let mut opts = realtime(seed);
    let seat = |key, name: &str, team| ReservedSeat {
        kind: None,
        key,
        name: name.to_owned(),
        team,
    };
    opts.reserved = vec![
        seat(KEYS[0], "Радиант", Team::Radiant),
        seat(KEYS[1], "ダイア", Team::Dire),
    ];
    opts.spectators = Spectators::Open;
    opts.replay = Some(replay);
    opts.empty_after = Some(Duration::from_millis(300));
    opts
}

/// The two seats, a spectator with Radiant's key, and an open spectator
/// watching through Dire's eyes, all in the match; the lobby line said by
/// Radiant before the seats were ready.
fn everyone_in(addr: std::net::SocketAddr) -> [TestClient; 4] {
    let mut seats = KEYS.map(|key| {
        let mut seat = TestClient::connect(addr);
        seat.hello(Role::Bot, Some(key));
        seat.welcome();
        seat
    });
    let mut keyed = TestClient::connect(addr);
    keyed.hello(Role::Spectator, Some(KEYS[0]));
    keyed.welcome();
    let mut open = TestClient::connect(addr);
    open.hello(Role::Spectator, None);
    open.welcome();
    seats[0].send(&say(Audience::All, LOBBY_TEXT));
    for seat in &mut seats {
        seat.send(&ClientMsg::PickHero {
            hero: HeroId::SHADOW_FIEND,
        });
        seat.send(&ClientMsg::SetReady(true));
    }
    let [radiant, dire] = seats;
    let mut all = [radiant, dire, keyed, open];
    for client in &mut all {
        client.recv_until(snapshot);
    }
    all[3].send(&ClientMsg::ViewAs {
        seat: Some(SlotId(1)),
    });
    all
}

/// Reads messages into `log` up to and with the first that is `done`.
fn read_until(
    client: &mut TestClient,
    log: &mut Vec<ServerMsg>,
    done: impl Fn(&ServerMsg) -> bool,
) {
    for _ in 0..1000 {
        let msg = client.recv();
        let last = done(&msg);
        log.push(msg);
        if last {
            return;
        }
    }
    panic!("the awaited message never came");
}

/// The records of a finished replay, header first.
fn replay_records(path: &std::path::Path) -> Vec<ReplayRecord> {
    for _ in 0..200 {
        thread::sleep(Duration::from_millis(50));
        // A stream the server has not finished does not unpack.
        let Some(bytes) = read_replay(path) else {
            continue;
        };
        let mut reader = FrameReader::new();
        reader.push(&bytes);
        return std::iter::from_fn(|| reader.next_message::<ReplayRecord>().expect("a record"))
            .collect();
    }
    panic!("the replay was never finished");
}

/// A team line reaches its side's seats and the spectators holding its
/// side's key, never the other side or one only borrowing its eyes; the
/// all line and the lobby's reach everyone; spectators talk among
/// themselves; every script survives the trip; and the replay of the whole
/// map keeps the seats' lines, team lines too, in the order said.
#[test]
fn a_team_line_reaches_its_side_only_and_the_replay_keeps_every_seat_line() {
    let path = std::env::temp_dir().join(format!("bota-chat-{}.brp", std::process::id()));
    let addr = start_server(chat_match(61, path.clone()));
    let [mut radiant, mut dire, mut keyed, mut open] = everyone_in(addr);
    // Each speaker hears its own line back before the next one speaks, so
    // the lines are said in this order.
    let mut told = [(); 4].map(|()| Vec::new());
    radiant.send(&say(Audience::Team, TEAM_TEXT));
    radiant.send(&say(Audience::Team, "a\u{202E}b"));
    read_until(&mut radiant, &mut told[0], |msg| {
        matches!(msg, ServerMsg::SayRefused { .. })
    });
    dire.send(&say(Audience::All, ALL_TEXT));
    read_until(&mut dire, &mut told[1], |msg| {
        matches!(msg, ServerMsg::Heard(_))
    });
    open.send(&say(Audience::Spectators, SPECTATOR_TEXT));
    read_until(
        &mut open,
        &mut told[3],
        |msg| matches!(msg, ServerMsg::Heard(heard) if heard.line.to == Audience::Spectators),
    );
    let wait = Duration::from_millis(600);
    for (client, log) in [&mut radiant, &mut dire, &mut keyed, &mut open]
        .into_iter()
        .zip(&mut told)
    {
        log.extend(drain(client, wait));
    }
    let [told_radiant, told_dire, told_keyed, told_open] = told;

    // The lobby line came before; what came now:
    assert_eq!(texts(&heard(&told_radiant)), [TEAM_TEXT, ALL_TEXT]);
    assert_eq!(refusals(&told_radiant), [SayRefusal::Unfit]);
    assert_eq!(
        texts(&heard(&told_dire)),
        [ALL_TEXT],
        "never the enemy's team line"
    );
    assert_eq!(
        texts(&heard(&told_keyed)),
        [TEAM_TEXT, ALL_TEXT, SPECTATOR_TEXT]
    );
    assert_eq!(
        texts(&heard(&told_open)),
        [ALL_TEXT, SPECTATOR_TEXT],
        "borrowed eyes never hear a side's team line"
    );
    for told in [&told_dire, &told_keyed, &told_open] {
        assert!(
            refusals(told).is_empty(),
            "a refusal is the speaker's alone"
        );
    }
    let team = &heard(&told_keyed)[0];
    assert_eq!(team.from, Speaker::Seat(SlotId(0)));
    assert_eq!(team.line.to, Audience::Team);
    assert!(team.tick.is_some_and(|tick| tick > 0));
    let spectator = &heard(&told_open)[1];
    assert_eq!(spectator.from, Speaker::Spectator("ignored".to_owned()));

    drop((radiant, dire, keyed, open));
    let kept: Vec<Heard> = replay_records(&path)
        .into_iter()
        .filter_map(|record| match record {
            ReplayRecord::Heard(heard) => Some(heard),
            _ => None,
        })
        .collect();
    let _ = std::fs::remove_file(&path);
    assert_eq!(texts(&kept), [LOBBY_TEXT, TEAM_TEXT, ALL_TEXT]);
    assert_eq!(kept[0].tick, None, "said in the lobby");
    assert_eq!(kept[1].line.to, Audience::Team);
}

/// A line over the rate, unfit, or to an audience the speaker may not reach
/// is refused to the speaker alone, which stays connected; a server may
/// keep seats to their side, or close chat to them.
#[test]
fn a_refused_line_reaches_nobody_and_the_speaker_stays() {
    for (seats, refused_to) in [
        (SeatChat::All, Audience::Spectators),
        (SeatChat::Team, Audience::All),
        (SeatChat::Off, Audience::Team),
    ] {
        let mut opts = realtime(62);
        opts.chat = ChatOpts {
            seats,
            ..ChatOpts::DEFAULT
        };
        let addr = start_server(opts);
        let (mut speaker, _) = join_as_bot(addr, "speaker");
        let (mut listener, _) = join_as_bot(addr, "listener");
        speaker.recv_until(snapshot);
        listener.recv_until(snapshot);
        let allowed = match seats {
            SeatChat::All => Some(Audience::All),
            SeatChat::Team => Some(Audience::Team),
            SeatChat::Off => None,
        };
        speaker.send(&say(refused_to, "nobody hears this"));
        speaker.send(&say(Audience::Team, "two\nlines"));
        for at in 0..4 {
            speaker.send(&say(
                allowed.unwrap_or(Audience::Team),
                &format!("line {at}"),
            ));
        }
        let told = drain(&mut speaker, Duration::from_millis(500));
        let heard_by_listener = heard(&drain(&mut listener, Duration::from_millis(200)));
        // The rate is taken first: five lines of burst, then one a second.
        let too_fast = SayRefusal::TooFast { retry_in_ms: 0 };
        let expected = match allowed {
            Some(_) => vec![SayRefusal::NotAllowed, SayRefusal::Unfit, too_fast],
            None => [[SayRefusal::NotAllowed; 5].as_slice(), &[too_fast]].concat(),
        };
        let refused: Vec<SayRefusal> = refusals(&told)
            .into_iter()
            .map(|reason| match reason {
                SayRefusal::TooFast { retry_in_ms } => {
                    assert!((1..=1000).contains(&retry_in_ms), "{retry_in_ms}");
                    SayRefusal::TooFast { retry_in_ms: 0 }
                }
                other => other,
            })
            .collect();
        assert_eq!(refused, expected, "{seats}");
        let passed = if allowed.is_some() { 3 } else { 0 };
        assert_eq!(
            heard(&told).len(),
            passed,
            "the speaker hears its own lines"
        );
        // The two seats play on different sides: only all-chat crosses.
        let crossed = if allowed == Some(Audience::All) { 3 } else { 0 };
        assert_eq!(heard_by_listener.len(), crossed, "{seats}");
        speaker.recv_until(snapshot);
    }
}

/// The ticks one seat is told of a lockstep match: every snapshot and its
/// events, the seat saying a line on every tick when `chatty`.
fn told_ticks(seed: u64, chatty: bool) -> Vec<ServerMsg> {
    let mut opts = realtime(seed);
    opts.mode = bota_proto::TickMode::Lockstep;
    opts.ack_timeout_ticks = 600;
    opts.dummy_seat = true;
    let addr = start_server(opts);
    let (mut seat, _) = join_as_bot(addr, "solo");
    let mut told = Vec::new();
    while told.len() < 120 {
        match seat.recv() {
            ServerMsg::Snapshot { view } => {
                let tick = view.tick;
                told.push(ServerMsg::Snapshot { view });
                if chatty {
                    seat.send(&say(Audience::All, &format!("тик {tick} 🎉")));
                    seat.send(&say(Audience::Team, "a\u{2066}b"));
                }
                seat.send(&ClientMsg::Ack { tick });
            }
            msg @ ServerMsg::Events { .. } => told.push(msg),
            _ => {}
        }
    }
    told
}

/// Chat is beside the match: the same seed with a line said on every tick
/// is the same match, snapshot for snapshot and event for event.
#[test]
fn a_match_flooded_with_chat_is_the_same_match() {
    assert_eq!(told_ticks(63, false), told_ticks(63, true));
}

/// A client that asks for compression is sent the match in compressed
/// parts, which unpack into the very messages a plain client is sent, and
/// chat plainly beside them.
#[test]
fn a_compressed_link_carries_the_match_in_parts_and_chat_plainly() {
    let addr = start_server(realtime(64));
    let mut packed = TestClient::connect(addr);
    packed.send(&ClientMsg::hello(
        Role::Bot,
        "packed",
        None,
        bota_core::WIRE_FINGERPRINT,
    ));
    packed.send(&ClientMsg::Compress(Codec::Deflate));
    packed.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    packed.send(&ClientMsg::SetReady(true));
    let (mut plain, _) = join_as_bot(addr, "plain");
    plain.recv_until(snapshot);
    plain.send(&say(Audience::All, ALL_TEXT));

    // Read the packed client's wire as it is, frame by frame.
    let mut outer = Vec::new();
    let mut frames = FrameReader::new();
    let mut buf = [0u8; 65536];
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let n = packed.stream.read(&mut buf).expect("read");
        assert!(n > 0, "the server hung up");
        frames.push(&buf[..n]);
        while let Some(msg) = frames.next_message::<ServerMsg>().expect("a frame") {
            outer.push(msg);
        }
    }
    let parts = outer
        .iter()
        .filter(|msg| matches!(msg, ServerMsg::Compressed(_)))
        .count();
    assert!(parts > 10, "the match comes in parts");
    assert!(
        !outer
            .iter()
            .any(|msg| matches!(msg, ServerMsg::Snapshot { .. })),
        "no snapshot goes out plainly once compression was asked for"
    );
    assert_eq!(texts(&heard(&outer)), [ALL_TEXT], "chat goes out plainly");

    let mut unpacker = bota_proto::Unpacker::new();
    let mut unpacked = Vec::new();
    let (mut wire_bytes, mut unpacked_bytes) = (0, 0);
    for msg in outer {
        if let ServerMsg::Compressed(part) = &msg {
            wire_bytes += part.len();
        }
        if let Some(msg) = unpacker.take(msg).expect("a part in order") {
            unpacked.push(msg);
        }
        while let Some(msg) = unpacker.next_message().expect("a whole part") {
            unpacked_bytes += bota_proto::payload_len(&msg).expect("it encodes");
            unpacked.push(msg);
        }
    }
    let views: Vec<WorldView> = unpacked.into_iter().filter_map(snapshot).collect();
    assert!(views.len() > 10, "{} snapshots", views.len());
    assert!(views.windows(2).all(|pair| pair[0].tick < pair[1].tick));
    assert!(
        unpacked_bytes > 5 * wire_bytes,
        "{unpacked_bytes} bytes unpacked from {wire_bytes}"
    );
}

fn ping(kind: PingKind, at: PingAt) -> ClientMsg {
    ClientMsg::Say(Line {
        to: Audience::Team,
        body: Body::Ping(Ping { kind, at }),
    })
}

/// A ping goes to the speaker's side alone, at a spot or at a unit its view
/// holds, named in its own names; one at a name the side does not hold, or
/// to everyone, is refused; the replay keeps it in the simulation's handles.
#[test]
fn a_ping_reaches_the_speakers_side_at_what_it_sees_and_the_replay_names_it_truly() {
    let path = std::env::temp_dir().join(format!("bota-ping-{}.brp", std::process::id()));
    let addr = start_server(chat_match(65, path.clone()));
    let [mut radiant, mut dire, mut keyed, mut open] = everyone_in(addr);
    let view = radiant.recv_until(snapshot);
    let hero = view.players[0].unit.expect("its hero stands");
    radiant.send(&ping(
        PingKind::Regular,
        PingAt::Spot(bota_proto::Vec2::from_ints(9000, 9000)),
    ));
    radiant.send(&ping(PingKind::Danger, PingAt::Unit(hero)));
    let nobody = EntityId {
        idx: 999_999,
        generation: 0,
    };
    radiant.send(&ping(PingKind::Danger, PingAt::Unit(nobody)));
    radiant.send(&ClientMsg::Say(Line {
        to: Audience::All,
        body: Body::Ping(Ping {
            kind: PingKind::Regular,
            at: PingAt::Spot(bota_proto::Vec2::from_ints(100, 100)),
        }),
    }));
    let mut told = Vec::new();
    read_until(&mut radiant, &mut told, |msg| {
        matches!(msg, ServerMsg::SayRefused { .. })
    });
    read_until(&mut radiant, &mut told, |msg| {
        matches!(msg, ServerMsg::SayRefused { .. })
    });
    told.extend(drain(&mut radiant, Duration::from_millis(300)));
    let pings = heard(&told);
    assert_eq!(pings.len(), 2, "{told:?}");
    assert_eq!(
        refusals(&told),
        [SayRefusal::NotAllowed, SayRefusal::NotAllowed]
    );
    assert_eq!(heard(&drain(&mut keyed, Duration::from_millis(300))), pings);
    for other in [&mut dire, &mut open] {
        assert!(heard(&drain(other, Duration::from_millis(300))).is_empty());
    }
    drop((radiant, dire, keyed, open));
    let kept: Vec<Heard> = replay_records(&path)
        .into_iter()
        .filter_map(|record| match record {
            ReplayRecord::Heard(heard) => Some(heard),
            _ => None,
        })
        .filter(|heard| matches!(heard.line.body, Body::Ping(_)))
        .collect();
    let _ = std::fs::remove_file(&path);
    assert_eq!(kept.len(), 2);
    let Body::Ping(Ping {
        at: PingAt::Unit(named),
        ..
    }) = kept[1].line.body
    else {
        panic!("the second ping is at the hero");
    };
    assert_ne!(
        named.generation, 0,
        "the replay holds the simulation's handle"
    );
}
