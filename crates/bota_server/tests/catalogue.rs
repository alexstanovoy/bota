//! Adding an item breaks nothing. A client built before the newest items
//! were appended connects to this server over a real socket, plays its match
//! on the items it holds and is never sold, given or told of the newer ones;
//! the replay of that match plays on this build, as a replay recorded before
//! an item was added must; and a client or a replay whose items differ from
//! this build's is refused.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use bota_client_core::{
    AckPolicy, Ingested, IssuedOrder, Link, Seat, SessionError, SessionOptions, Transport,
    UnpackedReplay, join,
};
use bota_core::Catalogue;
use bota_proto::{
    CataloguePrefix, Cheat, ClientMsg, FrameReader, HeroId, ItemId, MapId, Order, RejectReason,
    ReplayRecord, ServerMsg, TickMode,
};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

/// The catalogue of a build made before the Wind Lace was appended: it holds
/// neither the Wind Lace nor the Blood Grenade (sold from a stock) nor any item
/// appended after them, however many items this build holds.
fn older() -> Catalogue {
    Catalogue::first(ItemId::WIND_LACE.0).expect("a prefix")
}

/// The client library of another build: it says `told` where this build's
/// says its own catalogue, and is the same in everything else.
struct OtherBuild {
    link: Link,
    told: CataloguePrefix,
}

impl Transport for OtherBuild {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.link.recv()
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        match msg {
            ClientMsg::Catalogue(_) => self.link.send(&ClientMsg::Catalogue(self.told)),
            _ => self.link.send(msg),
        }
    }
}

fn opts(replay: PathBuf) -> ServerOpts {
    seats(replay, 1)
}

fn seats(replay: PathBuf, players: u8) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players,
        replay: Some(replay),
        replay_max_bytes: None,
        seed: Some(11),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(1),
        cheats: true,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: false,
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
        empty_after: Some(Duration::from_secs(1)),
    }
}

fn serve(opts: ServerOpts) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || {
        let _ = run(listener, opts);
    });
    (address, server)
}

/// A client of another build that told `told`, seated as a lone Shadow Fiend.
fn other_build(address: &str, told: CataloguePrefix) -> std::io::Result<(OtherBuild, Seat)> {
    let link = Link::connect(address, Duration::from_secs(60))?;
    let mut client = OtherBuild { link, told };
    let seated = join(&mut client, "elder", HeroId::SHADOW_FIEND, None)?;
    Ok((client, Seat::new(seated, SessionOptions::default())))
}

/// Ticks an order is given to be refused; one not refused by then was taken.
const ANSWER_TICKS: u32 = 15;

/// The orders the older client gives, one at a time, once the match runs:
/// gold to spend, then a newer item to buy, a newer item to be given, and an
/// item it holds to buy. Every message is checked for the newer items. What
/// the server answered to each comes back, its refusal or none, with the
/// ticks the client saw.
fn play(client: &mut OtherBuild, seat: &mut Seat) -> (Vec<Option<RejectReason>>, u32) {
    let orders = [
        Order::Cheat {
            cheat: Cheat::Gold { amount: 5000 },
        },
        Order::Buy {
            item: ItemId::WIND_LACE,
        },
        Order::Cheat {
            cheat: Cheat::Item {
                item: ItemId::BLOOD_GRENADE,
            },
        },
        Order::Buy {
            item: ItemId::CIRCLET,
        },
    ];
    let mut answers: Vec<Option<RejectReason>> = Vec::new();
    // The tick the order awaiting its answer was given on.
    let mut given: Option<u32> = None;
    let mut ticks = 0;
    while answers.len() < orders.len() {
        let msg = client.recv().expect("the link").expect("open");
        assert_older(&msg);
        match seat.ingest(msg, &mut ()).expect("a sound stream") {
            Ingested::Rejected { reason, .. } => {
                answers.push(Some(reason));
                given = None;
            }
            Ingested::TickComplete { .. } => {
                ticks += 1;
                assert!(ticks < 2_000, "the orders were never answered");
                let session = seat.session_mut().expect("started");
                if given.is_some_and(|at| ticks >= at + ANSWER_TICKS) {
                    answers.push(None);
                    given = None;
                }
                if given.is_none() && answers.len() < orders.len() {
                    let order = IssuedOrder::hero(orders[answers.len()]);
                    let outgoing = session.issue(order).expect("issued");
                    client.send(&outgoing.message()).expect("sent");
                    given = Some(ticks);
                }
                if let Some(tick) = session.end_turn() {
                    client.send(&ClientMsg::Ack { tick }).expect("acked");
                }
            }
            Ingested::Over { .. } => panic!("the match ended early"),
            _ => {}
        }
    }
    client.send(&ClientMsg::LeaveGame).expect("left");
    (answers, ticks)
}

/// Nothing past the older catalogue is told to its client: the match plays
/// the older catalogue, its shop lists and stocks none of the newer items.
fn assert_older(msg: &ServerMsg) {
    match msg {
        ServerMsg::MatchStart { info } => {
            assert_eq!(
                info.catalogue,
                older().prefix(),
                "the match plays the older catalogue"
            );
            assert!(info.shop.iter().all(|entry| older().holds(entry.id)));
        }
        ServerMsg::Snapshot { view } => {
            assert!(view.stock.iter().all(|stock| older().holds(stock.item)));
            assert!(view.units.iter().all(|unit| {
                unit.items
                    .iter()
                    .flatten()
                    .all(|item| older().holds(item.id))
            }));
        }
        _ => {}
    }
}

/// The records of the replay a server wrote, header first.
fn replay_records(path: &Path) -> Vec<ReplayRecord> {
    let file = std::fs::File::open(path).expect("the replay exists");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut UnpackedReplay::new(file), &mut bytes)
        .expect("the replay is finished");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    std::iter::from_fn(|| reader.next_message::<ReplayRecord>().expect("a record")).collect()
}

/// Plays a replay's match on this build's client, as a player of replays
/// does: what it refuses, if anything.
fn play_replay(records: &[ReplayRecord]) -> Result<u32, SessionError> {
    let options = SessionOptions {
        ack: AckPolicy::Never,
        ..SessionOptions::default()
    };
    let mut seat = Seat::watching(options);
    let mut ticks = 0;
    for record in records {
        if let ReplayRecord::Msg(msg) = record
            && let Ingested::Snapshot { .. } = seat.ingest(msg.clone(), &mut ())?
        {
            ticks += 1;
        }
    }
    Ok(ticks)
}

#[test]
fn a_client_built_before_newer_items_plays_without_them_and_its_replay_plays_here() {
    let path = std::env::temp_dir().join(format!("bota-catalogue-{}.brp", std::process::id()));
    let (address, server) = serve(opts(path.clone()));
    let (mut client, mut seat) = other_build(&address, older().prefix()).expect("seated");
    let (answers, seen) = play(&mut client, &mut seat);
    assert_eq!(
        answers,
        vec![
            None,
            Some(RejectReason::UnknownItem),
            Some(RejectReason::BadCheat),
            None,
        ],
        "an item the client does not hold is neither sold nor given; one it holds is"
    );
    server.join().expect("the server ends");
    let records = replay_records(&path);
    let _ = std::fs::remove_file(&path);
    let ticks = play_replay(&records).expect("the replay of the older catalogue plays here");
    assert!(
        ticks >= seen,
        "the whole match plays: {ticks} of {seen} ticks"
    );
    let mut differing = records.clone();
    for record in &mut differing {
        if let ReplayRecord::Msg(ServerMsg::MatchStart { info }) = record {
            info.catalogue.digest ^= 1;
        }
    }
    assert!(
        matches!(
            play_replay(&differing),
            Err(SessionError::OtherCatalogue { .. })
        ),
        "a replay whose items differ from this build's is refused"
    );
}

#[test]
fn a_client_whose_items_differ_is_refused() {
    let path = std::env::temp_dir().join(format!("bota-catalogue-x-{}.brp", std::process::id()));
    let (address, server) = serve(opts(path.clone()));
    let mut changed = Catalogue::FULL.prefix();
    changed.digest ^= 1;
    // The catalogue follows the hello, so its refusal follows the welcome.
    let (mut differing, _) = other_build(&address, changed).expect("welcomed");
    let refusal = std::iter::from_fn(|| differing.recv().ok().flatten())
        .find(|msg| matches!(msg, ServerMsg::Refused { .. }));
    assert_eq!(
        refusal,
        Some(ServerMsg::Refused {
            reason: RejectReason::CatalogueMismatch
        }),
        "a client whose items differ is refused"
    );
    // The seat stays open for one that agrees, which ends the match.
    let (mut client, _) = other_build(&address, older().prefix()).expect("seated");
    client.send(&ClientMsg::LeaveGame).expect("left");
    server.join().expect("the server ends");
    let _ = std::fs::remove_file(&path);
}

/// A client of this build sitting down with one of an older build: before the start its lobby
/// is told the older catalogue the two share, and the match it plays is on that catalogue, so
/// its shop offers no item past it, neither while the shop is open before the start nor after.
#[test]
fn a_client_sharing_a_match_with_an_older_one_offers_only_the_items_both_hold() {
    let path = std::env::temp_dir().join(format!("bota-catalogue-s-{}.brp", std::process::id()));
    let (address, server) = serve(seats(path.clone(), 2));
    let (mut link, seated) =
        Link::join(&address, "newer", HeroId::SHADOW_FIEND, None).expect("seated");
    let mut lobby = bota_client_core::Lobby::default();
    let (mut older_client, older_seated) = {
        let link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
        let mut client = OtherBuild {
            link,
            told: older().prefix(),
        };
        let seated = join(&mut client, "elder", HeroId::PUDGE, None).expect("seated");
        (client, seated)
    };
    let elder = thread::spawn(move || {
        let _ = bota_client_core::play(&mut older_client, older_seated, &mut (), |_| Vec::new());
    });
    let mut seat = Seat::new(seated, SessionOptions::default());
    let mut ticks = 0;
    while ticks < 30 {
        let msg = link.recv().expect("the link").expect("open");
        lobby.observe(&msg);
        if let ServerMsg::MatchStart { info } = &msg {
            assert_eq!(
                lobby.catalogue,
                Some(older()),
                "before the start the lobby tells the items both seats hold"
            );
            assert_eq!(info.catalogue, older().prefix());
        }
        if let Ingested::TickComplete { .. } = seat.ingest(msg, &mut ()).expect("a sound stream") {
            ticks += 1;
            let session = seat.session_mut().expect("started");
            let catalogue = session.statics().catalogue();
            assert_eq!(catalogue, older(), "the match plays the items both hold");
            assert!(
                bota_client_core::sold_items(catalogue).all(|item| older().holds(item)),
                "the shop offers no item past them"
            );
            if let Some(tick) = session.end_turn() {
                link.send(&ClientMsg::Ack { tick }).expect("acked");
            }
        }
    }
    link.send(&ClientMsg::LeaveGame).expect("left");
    server.join().expect("the server ends");
    elder.join().expect("the older client ends");
    let _ = std::fs::remove_file(&path);
}
