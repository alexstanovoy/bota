//! One player alone: the other side has no seat, so no hero, and the match runs on as the map's
//! own rules have it. A lone player on Map2 wins by felling the enemy's first tower, and on the
//! full map by felling its Ancient; the empty side never counts as defeated, nor its seat as
//! one that never came. Cheats make it quick (a level 30 hero that cannot die, in a match
//! with the clock's start skipped).

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use std::path::{Path, PathBuf};

use bota_client_core::{IssuedOrder, Link, UnpackedReplay};
use bota_proto::{
    Cheat, FrameReader, HeroId, ItemId, MapId, Order, ReplayRecord, ServerMsg, Target, Team,
    TickMode, UnitKind, Vec2,
};
use bota_replay::meta::{MetaRecord, Who, decode, stat};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

fn opts(map: MapId, files: Option<(PathBuf, PathBuf)>) -> ServerOpts {
    let scripted = files.is_some();
    let (replay, meta) = files.map_or((None, None), |(replay, meta)| (Some(replay), Some(meta)));
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
        meta,
        meta_trailer: true,
        seed: Some(5),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map,
        cheats: true,
        immortal_structures: false,
        // The scripted match fells a hero.
        immortal_heroes: !scripted,
        objectives: None,
        runes: None,
        dummy_seat: false,
        seated_start: false,
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
        empty_after: Some(Duration::from_secs(5)),
    }
}

/// A lone player's match to its end: levels, the game started, then an attack-move on the enemy
/// fountain, until the map ends it.
fn alone_on(map: MapId) -> (bota_client_core::Outcome, u32) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || run(listener, opts(map, None)));
    let target = bota_core::fountain_pos(bota_core::map_of(map), Team::Dire);
    let mut decisions = 0u32;
    let outcome = Link::play(&address, "alone", HeroId::SHADOW_FIEND, |session| {
        decisions += 1;
        let hero = |order| IssuedOrder::hero(order);
        match decisions {
            1 => vec![hero(Order::Cheat {
                cheat: Cheat::Levels { count: 29 },
            })],
            2 => vec![hero(Order::Cheat {
                cheat: Cheat::StartGame,
            })],
            _ if decisions.is_multiple_of(30) && session.current().is_some() => {
                vec![hero(Order::Attack {
                    target: Target::Pos(target),
                })]
            }
            _ => Vec::new(),
        }
    })
    .expect("the match is played to its end");
    let _ = server.join();
    (outcome, decisions)
}

#[test]
fn a_player_alone_on_map2_wins_by_the_first_tower() {
    let (outcome, _) = alone_on(MapId(2));
    assert_eq!(outcome.team, Team::Radiant);
    // Not at once, as an empty side lost would have it: it takes a hero's walk to a tower.
    assert!(outcome.ticks > 600, "{outcome:?}");
    assert_eq!(outcome.winner, Team::Radiant, "the tower fell: {outcome:?}");
}

#[test]
fn a_player_alone_on_the_full_map_wins_by_the_ancient() {
    let (outcome, _) = alone_on(MapId(0));
    assert_eq!(
        outcome.winner,
        Team::Radiant,
        "the Ancient fell: {outcome:?}"
    );
}

/// The files one scripted match left.
struct Left {
    replay: PathBuf,
    meta: PathBuf,
}

/// The records of a finished replay file, header first.
fn records(path: &Path) -> Vec<ReplayRecord> {
    let file = std::fs::File::open(path).expect("the file exists");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut UnpackedReplay::new(file), &mut bytes)
        .expect("a finished stream, a trailer after it skipped");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    std::iter::from_fn(|| reader.next_message::<ReplayRecord>().expect("a record")).collect()
}

const BOUGHT: [ItemId; 3] = [ItemId::CLARITY, ItemId::TANGO, ItemId::HEALING_SALVE];

/// A lone player on Map 2 buys three items, goes to level 30, fells a hero the game created
/// beside it, and walks on the enemy's fountain until the map ends the match; the server leaves
/// its replay (with the metadata at its end) and the metadata file.
fn scripted() -> Left {
    let dir = std::env::temp_dir();
    let id = std::process::id();
    let left = Left {
        replay: dir.join(format!("bota-meta-{id}.brp")),
        meta: dir.join(format!("bota-meta-{id}.meta")),
    };
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let files = (left.replay.clone(), left.meta.clone());
    let server = thread::spawn(move || run(listener, opts(MapId(2), Some(files))));
    let map = bota_core::map_of(MapId(2));
    let middle = Vec2::from_ints(7500, 7500);
    let enemy_fountain = bota_core::fountain_pos(map, Team::Dire);
    let mut decisions = 0u32;
    let outcome = Link::play(&address, "meta", HeroId::SHADOW_FIEND, |session| {
        decisions += 1;
        let hero = IssuedOrder::hero;
        let cheat = |cheat| hero(Order::Cheat { cheat });
        match decisions {
            1 => return vec![cheat(Cheat::Levels { count: 29 })],
            2 => return vec![cheat(Cheat::StartGame)],
            3 => return BOUGHT.map(|item| hero(Order::Buy { item })).to_vec(),
            4 => return vec![cheat(Cheat::Teleport { at: middle })],
            5 => {
                return vec![cheat(Cheat::CreateHero {
                    hero: HeroId::SHADOW_FIEND,
                    enemy: true,
                    at: middle + Vec2::from_ints(200, 0),
                })];
            }
            _ => {}
        }
        let Some(view) = session.current() else {
            return Vec::new();
        };
        let killed = session
            .slot()
            .and_then(|slot| view.player(slot))
            .is_some_and(|me| me.kills >= 1);
        let foe = view
            .units
            .iter()
            .find(|u| u.kind == UnitKind::Hero && u.team == Team::Dire);
        match foe {
            Some(foe) if !killed => vec![hero(Order::Attack {
                target: Target::Unit(foe.id),
            })],
            _ if killed && decisions.is_multiple_of(30) => vec![hero(Order::Attack {
                target: Target::Pos(enemy_fountain),
            })],
            _ => Vec::new(),
        }
    })
    .expect("the match is played to its end");
    assert_eq!(outcome.winner, Team::Radiant, "{outcome:?}");
    server.join().expect("the server ends").expect("it ran");
    left
}

#[test]
fn a_match_leaves_its_metadata_in_a_file_and_in_its_replay_and_the_recorder_writes_the_same() {
    let left = scripted();
    let meta_file = std::fs::read(&left.meta).expect("the meta file");
    let records = records(&left.replay);
    let mut replay = std::fs::File::open(&left.replay).expect("the replay");
    // The replay carries the file at its end, byte for byte, and plays as before.
    let carried = bota_replay::trailer::find(&mut replay).expect("reads");
    assert_eq!(carried.as_deref(), Some(&meta_file[..]));
    // A tool running the recorder over the replay afterwards writes the very same bytes.
    let rebuilt = bota_client_core::meta_of_replay(std::fs::File::open(&left.replay).unwrap())
        .expect("the replay records");
    assert_eq!(
        rebuilt, meta_file,
        "the live recorder and the offline one agree"
    );

    let meta = decode(&meta_file).expect("a file");
    assert!(meta.complete && meta.skipped == 0);
    assert_eq!(meta.header.wire_fingerprint, bota_core::WIRE_FINGERPRINT);
    let over = records
        .iter()
        .find_map(|r| match r {
            ReplayRecord::Msg(ServerMsg::MatchOver { stats, .. }) => Some(stats.clone()),
            _ => None,
        })
        .expect("the match ended");
    let pregame = meta.header.pregame_ticks;
    let last_tick = records
        .iter()
        .filter_map(|r| match r {
            ReplayRecord::Msg(ServerMsg::Snapshot { view }) => Some(view.game_tick()),
            _ => None,
        })
        .next_back()
        .expect("snapshots");
    let sample = |slot: u8| -> Vec<&Vec<(u16, i64)>> {
        meta.records
            .iter()
            .filter_map(|r| match r {
                MetaRecord::Sample { slot: s, stats, .. } if *s == slot => Some(stats),
                _ => None,
            })
            .collect()
    };
    let mine = sample(0);
    let minutes = (last_tick - pregame) / (60 * bota_core::rules::TICKS_PER_SECOND);
    // One sample for the start, one for every whole game minute after, and the last.
    assert_eq!(mine.len() as u32, minutes + 1 + 1, "a sample a minute");
    let value = |stats: &Vec<(u16, i64)>, id| stats.iter().find(|(s, _)| *s == id).map(|v| v.1);
    let last = mine.last().expect("a last sample");
    let own = over.slots.iter().find(|s| s.slot.0 == 0).expect("stats");
    assert_eq!(value(last, stat::KILLS), Some(1));
    assert_eq!(value(last, stat::DEATHS), Some(0));
    assert_eq!(value(last, stat::LEVEL), Some(30));
    for (id, told) in [
        (stat::KILLS, i64::from(own.kills)),
        (stat::DEATHS, i64::from(own.deaths)),
        (stat::ASSISTS, i64::from(own.assists)),
        (stat::LAST_HITS, i64::from(own.last_hits)),
        (stat::DENIES, i64::from(own.denies)),
        (stat::GOLD_EARNED, i64::from(own.net_worth)),
        (stat::HERO_DAMAGE, i64::from(own.hero_damage)),
        (stat::STRUCTURE_DAMAGE, i64::from(own.structure_damage)),
    ] {
        assert_eq!(
            value(last, id),
            Some(told),
            "stat {id} as the match told it"
        );
    }
    assert!(own.hero_damage > 0 && own.structure_damage > 0);
    // Nothing was sold and the player never died: what it holds in gold and goods is what it
    // earned, which the server counts on its own.
    assert_eq!(value(last, stat::NET_WORTH), Some(i64::from(own.net_worth)));
    let bought: Vec<u16> = meta
        .records
        .iter()
        .filter_map(|r| match r {
            MetaRecord::Purchase { slot: 0, item, .. } => Some(*item),
            _ => None,
        })
        .collect();
    assert_eq!(bought, BOUGHT.map(|item| item.0));
    let kills: Vec<_> = meta
        .records
        .iter()
        .filter_map(|r| match r {
            MetaRecord::Kill { victim, killer, .. } => Some((*victim, *killer)),
            _ => None,
        })
        .collect();
    assert_eq!(kills.len(), 1, "one hero fell");
    assert_eq!(
        kills[0],
        (Who::Other, Some(Who::Hero(0))),
        "a hero made for no seat fell to seat 0"
    );
    let held: Vec<u16> = meta
        .records
        .iter()
        .find_map(|r| match r {
            MetaRecord::Inventory { slot: 0, items, .. } => {
                Some(items.iter().flatten().copied().collect())
            }
            _ => None,
        })
        .expect("what the hero carried");
    assert_eq!(held, BOUGHT.map(|item| item.0));
    assert!(meta.records.iter().any(|r| matches!(
        r,
        MetaRecord::Structure { kind, team: 1, .. } if *kind == UnitKind::Tower as u8
    )));
    assert!(matches!(
        meta.records.last(),
        Some(MetaRecord::End {
            winner: Some(0),
            ..
        })
    ));
    // The player of this build plays a replay with the trailer to its end, as it plays one
    // without.
    let mut player = bota_client_core::ReplayPlayer::load(&left.replay).expect("opens");
    player.advance_ticks(1.0e9);
    let mut played = 0;
    while !player.finished() && player.error().is_none() && played < 10_000_000 {
        played += player.poll(0.0).len();
    }
    assert!(player.error().is_none(), "{:?}", player.error());
    assert!(player.finished() && played > 0);
    for path in [&left.replay, &left.meta] {
        let _ = std::fs::remove_file(path);
    }
}
