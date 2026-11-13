//! A missed attack end to end: two players in a lockstep cheat match on the Dota map over real
//! sockets, one hero standing where the scene puts it (by `-teleport`) and attacking the other's
//! (a real attack order), a spectator watching the whole map and the replay written. Swinging up
//! from the mid river at the bank, a ranged hero misses Dota's quarter of its blows; every miss is
//! told to the attacker's seat, the target's seat, the spectator and the replay, takes no health
//! and lays nothing its attack carries. The same swings across level ground never miss, and an
//! evasion item makes them miss there.

use std::collections::VecDeque;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use bota_client_core::{
    Ingested, IssuedOrder, Link, Seat, SessionOptions, Transport, UnpackedReplay, greet,
};
use bota_proto::{
    Cheat, ClientMsg, EntityId, EventKind, FrameReader, HeroId, ItemId, MapId, ModifierSpec, Order,
    ReplayRecord, Role, ServerMsg, Target, Team, TickMode, UnitKind, UnitView, Vec2, WorldView,
};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

/// The blows each scene counts: every one either lands or misses.
const BLOWS: usize = 40;
/// Ticks a scene may take, the setting up included.
const MAX_TICKS: u32 = 6_000;
/// In the mid river (elevation tier 0) and up its Dire bank (tier 1), in a ranged hero's reach of
/// each other; and a second river spot as far from the first, level with it.
const RIVER: Vec2 = Vec2::from_ints(8750, 8880);
const BANK: Vec2 = Vec2::from_ints(9150, 9150);
const RIVER_TOO: Vec2 = Vec2::from_ints(8950, 9080);

fn opts(replay: PathBuf) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 2,
        replay: Some(replay),
        replay_max_bytes: None,
        meta: None,
        meta_trailer: false,
        seed: Some(5),
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
        ack_timeout_ticks: 100_000,
        idle_timeout: None,
        order_delays: Vec::new(),
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Open,
        heroes: None,
        abandon_unjoined: false,
        empty_after: Some(Duration::from_secs(1)),
    }
}

/// Where the two heroes stand and what they carry.
struct Scene {
    name: &'static str,
    attacker_at: Vec2,
    target_at: Vec2,
    attacker_items: &'static [ItemId],
    target_items: &'static [ItemId],
    /// An item the attacker is handed once it has been told of so many blows.
    handed_after: Option<(usize, ItemId)>,
}

/// One blow of the attacker's at the target, as a seat was told of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Blow {
    Landed,
    Missed,
}

/// What one seat saw of the attacker's blows at the target, tick by tick.
#[derive(Default)]
struct Told {
    /// Each blow with the tick it was told on, in order.
    blows: Vec<(u32, Blow)>,
    /// Health of the target at the end of each tick, by tick.
    target_hp: Vec<(u32, i32)>,
    /// Armor of the target at the end of each tick, by tick.
    target_armor: Vec<(u32, i32)>,
}

impl Told {
    fn misses(&self) -> usize {
        self.blows
            .iter()
            .filter(|(_, b)| *b == Blow::Missed)
            .count()
    }
}

/// A seat and the orders it still has to give, one a tick.
struct Player {
    link: Link,
    seat: Seat,
    pending: VecDeque<Order>,
    told: Told,
}

impl Player {
    fn join(address: &str, name: &str) -> Player {
        let (link, seated) = Link::join(address, name, HeroId::SHADOW_FIEND, None).expect("a seat");
        Player {
            link,
            seat: Seat::new(seated, SessionOptions::default()),
            pending: VecDeque::new(),
            told: Told::default(),
        }
    }

    /// Reads one whole tick, gives the next pending order and acknowledges it; the tick's events
    /// come back with the view the tick ended on.
    fn turn(&mut self) -> (Vec<EventKind>, WorldView) {
        let mut events = Vec::new();
        loop {
            let msg = self.link.recv().expect("the link").expect("open");
            if let ServerMsg::Events { events: told, .. } = &msg {
                events.extend(told.iter().cloned());
            }
            match self.seat.ingest(msg, &mut ()).expect("a sound stream") {
                Ingested::TickComplete { .. } => break,
                Ingested::Rejected { reason, .. } => panic!("an order was refused: {reason:?}"),
                Ingested::Over { .. } => panic!("the match ended early"),
                _ => {}
            }
        }
        let session = self.seat.session_mut().expect("started");
        let view = session.current().expect("a view").clone();
        if let Some(order) = self.pending.pop_front() {
            let outgoing = session.issue(IssuedOrder::hero(order)).expect("issued");
            self.link.send(&outgoing.message()).expect("sent");
        }
        if let Some(tick) = session.end_turn() {
            self.link.send(&ClientMsg::Ack { tick }).expect("acked");
        }
        (events, view)
    }
}

fn hero_of(view: &WorldView, team: Team) -> Option<&UnitView> {
    view.units
        .iter()
        .find(|unit| unit.kind == UnitKind::Hero && unit.team == team)
}

/// The blows from `source` at `target` among one tick's events.
fn blows_in(events: &[EventKind], source: EntityId, target: EntityId) -> Vec<Blow> {
    events
        .iter()
        .filter_map(|event| match event {
            EventKind::Damaged {
                source: Some(by),
                target: at,
                ..
            } if *by == source && *at == target => Some(Blow::Landed),
            EventKind::Missed {
                source: Some(by),
                target: at,
            } if *by == source && *at == target => Some(Blow::Missed),
            _ => None,
        })
        .collect()
}

fn record(told: &mut Told, tick: u32, blows: Vec<Blow>, target: Option<&UnitView>) {
    told.blows
        .extend(blows.into_iter().map(|blow| (tick, blow)));
    if let Some(target) = target {
        told.target_hp.push((tick, target.hp));
        told.target_armor.push((tick, target.armor.to_int()));
    }
}

/// A whole-map spectator's count of the misses at a hero, read on a thread of its own until the
/// match ends.
fn watch(address: String) -> thread::JoinHandle<usize> {
    thread::spawn(move || {
        let mut link = Link::connect(&address, Duration::from_secs(10)).expect("connected");
        greet(&mut link, Role::Spectator, "watcher", None).expect("welcomed");
        let mut heroes = Vec::new();
        let mut misses = 0;
        while let Ok(Some(msg)) = link.recv() {
            match msg {
                ServerMsg::Snapshot { view } => {
                    heroes = view
                        .units
                        .iter()
                        .filter(|unit| unit.kind == UnitKind::Hero)
                        .map(|unit| unit.id)
                        .collect();
                }
                ServerMsg::Events { events, .. } => {
                    misses += events
                        .iter()
                        .filter(|event| {
                            matches!(event, EventKind::Missed { source: Some(_), target }
                                if heroes.contains(target))
                        })
                        .count();
                }
                ServerMsg::MatchOver { .. } => break,
                _ => {}
            }
        }
        misses
    })
}

/// The misses at a hero the replay's fogless stream tells of.
fn replay_misses(path: &Path) -> usize {
    let file = std::fs::File::open(path).expect("the replay exists");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut UnpackedReplay::new(file), &mut bytes)
        .expect("the replay is finished");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    let mut heroes = Vec::new();
    let mut misses = 0;
    while let Some(record) = reader.next_message::<ReplayRecord>().expect("a record") {
        match record {
            ReplayRecord::Msg(ServerMsg::Snapshot { view }) => {
                heroes = view
                    .units
                    .iter()
                    .filter(|unit| unit.kind == UnitKind::Hero)
                    .map(|unit| unit.id)
                    .collect();
            }
            ReplayRecord::Msg(ServerMsg::Events { events, .. }) => {
                misses += events
                    .iter()
                    .filter(|event| {
                        matches!(event, EventKind::Missed { source: Some(_), target }
                            if heroes.contains(target))
                    })
                    .count();
            }
            _ => {}
        }
    }
    misses
}

/// What a scene came to: each seat's view of it, the spectator's and the replay's miss counts.
struct Played {
    attacker: Told,
    target: Told,
    spectator_misses: usize,
    replay_misses: usize,
}

/// The attacker's setup: no creeps, both sides seeing everything (the river does not see up its
/// bank: in a match a creep or a ward on the high ground lends that sight), nothing hurts the
/// attacker, its blows a quarter as strong (so the target outlives the count), its items, then
/// its spot.
fn attacker_setup(scene: &Scene) -> VecDeque<Order> {
    let weak = ModifierSpec {
        physical_damage: ModifierSpec::MIN_SCALE,
        ..ModifierSpec::NOMINAL
    };
    let mut cheats = vec![
        Cheat::CreepSpawning { on: false },
        Cheat::KillCreeps {
            which: bota_proto::CreepSide::All,
        },
        Cheat::AllVision { on: true },
        Cheat::Invulnerable { on: true },
        Cheat::ApplyModifier {
            target: Target::None,
            spec: weak,
            ticks: bota_proto::MAX_MODIFIER_TICKS,
        },
    ];
    cheats.extend(
        scene
            .attacker_items
            .iter()
            .map(|&item| Cheat::Item { item }),
    );
    cheats.push(Cheat::Teleport {
        at: scene.attacker_at,
    });
    cheats
        .into_iter()
        .map(|cheat| Order::Cheat { cheat })
        .collect()
}

fn target_setup(scene: &Scene) -> VecDeque<Order> {
    let mut cheats: Vec<Cheat> = scene
        .target_items
        .iter()
        .map(|&item| Cheat::Item { item })
        .collect();
    cheats.push(Cheat::Teleport {
        at: scene.target_at,
    });
    cheats
        .into_iter()
        .map(|cheat| Order::Cheat { cheat })
        .collect()
}

fn play(scene: &Scene) -> Played {
    // Each play its own folder: the tests run side by side, several on the same scene.
    static PLAYS: AtomicU32 = AtomicU32::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "misses-{}-{}-{}",
        scene.name,
        std::process::id(),
        PLAYS.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let replay = dir.join("match.brp");
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let options = opts(replay.clone());
    let server = thread::spawn(move || run(listener, options));
    let spectator = watch(address.clone());
    let mut attacker = Player::join(&address, "attacker");
    let mut target = Player::join(&address, "target");
    attacker.pending = attacker_setup(scene);
    target.pending = target_setup(scene);
    let mut attacking = false;
    let mut handed = false;
    let mut settled_ticks = 0;
    for _ in 0..MAX_TICKS {
        let (attacker_events, attacker_view) = attacker.turn();
        let (target_events, target_view) = target.turn();
        let (Some(me), Some(foe)) = (
            hero_of(&attacker_view, Team::Radiant),
            hero_of(&attacker_view, Team::Dire),
        ) else {
            continue;
        };
        let (me, foe) = (me.id, foe.id);
        if !attacking {
            let placed = attacker.pending.is_empty() && target.pending.is_empty();
            settled_ticks = if placed { settled_ticks + 1 } else { 0 };
            // A few ticks for the last teleport to be felt before the attack is ordered.
            if settled_ticks == 5 {
                attacker.pending.push_back(Order::Attack {
                    target: Target::Unit(foe),
                });
                attacking = true;
            }
            continue;
        }
        record(
            &mut attacker.told,
            attacker_view.tick,
            blows_in(&attacker_events, me, foe),
            attacker_view.units.iter().find(|unit| unit.id == foe),
        );
        let (theirs, mine) = (
            hero_of(&target_view, Team::Radiant).map(|unit| unit.id),
            hero_of(&target_view, Team::Dire),
        );
        if let (Some(theirs), Some(mine)) = (theirs, mine) {
            record(
                &mut target.told,
                target_view.tick,
                blows_in(&target_events, theirs, mine.id),
                Some(mine),
            );
        }
        if let Some((after, item)) = scene.handed_after
            && attacker.told.blows.len() == after
            && !handed
        {
            attacker.pending.push_back(Order::Cheat {
                cheat: Cheat::Item { item },
            });
            handed = true;
        }
        if attacker.told.blows.len() >= BLOWS && target.told.blows.len() >= BLOWS {
            break;
        }
    }
    assert!(
        attacker.told.blows.len() >= BLOWS,
        "{}: only {} blows were told within {MAX_TICKS} ticks",
        scene.name,
        attacker.told.blows.len()
    );
    attacker.told.blows.truncate(BLOWS);
    target.told.blows.truncate(BLOWS);
    for player in [&mut attacker, &mut target] {
        player.link.send(&ClientMsg::LeaveGame).expect("left");
    }
    drop(attacker.link);
    drop(target.link);
    let spectator_misses = spectator.join().expect("the spectator's thread");
    server
        .join()
        .expect("the server's thread")
        .expect("the match ended");
    let replay_misses = replay_misses(&replay);
    let _ = std::fs::remove_dir_all(&dir);
    Played {
        attacker: attacker.told,
        target: target.told,
        spectator_misses,
        replay_misses,
    }
}

/// The ticks a miss was told on with no blow landing on the same tick.
fn miss_only_ticks(told: &Told) -> Vec<u32> {
    told.blows
        .iter()
        .filter(|(tick, blow)| {
            *blow == Blow::Missed
                && !told
                    .blows
                    .iter()
                    .any(|(other, b)| other == tick && *b == Blow::Landed)
        })
        .map(|(tick, _)| *tick)
        .collect()
}

fn value_at(series: &[(u32, i32)], tick: u32) -> Option<i32> {
    series.iter().find(|(at, _)| *at == tick).map(|(_, v)| *v)
}

/// Blight Stone is handed over after the third blow: seed 5's fourth uphill blow misses (the
/// pinned run below), so the first blow the stone rides on is a miss.
const UPHILL: Scene = Scene {
    name: "uphill",
    attacker_at: RIVER,
    target_at: BANK,
    attacker_items: &[],
    target_items: &[],
    handed_after: Some((3, ItemId::BLIGHT_STONE)),
};

#[test]
fn a_ranged_hero_swinging_up_from_the_river_misses_a_quarter_and_both_seats_are_told() {
    let played = play(&UPHILL);
    let misses = played.attacker.misses();
    // Seed 5's run of Dota's pseudo-random 25%: exact, so nothing about it can drift unseen.
    assert_eq!(
        misses, 10,
        "uphill misses over {BLOWS} blows: {:?}",
        played.attacker.blows
    );
    assert_eq!(
        played
            .target
            .blows
            .iter()
            .map(|(_, b)| *b)
            .collect::<Vec<_>>(),
        played
            .attacker
            .blows
            .iter()
            .map(|(_, b)| *b)
            .collect::<Vec<_>>(),
        "the target's seat is told of the same blows, misses included, in the same order"
    );
}

#[test]
fn a_miss_takes_no_health_and_lays_no_on_hit_debuff() {
    let played = play(&UPHILL);
    let told = &played.attacker;
    let alone = miss_only_ticks(told);
    assert!(!alone.is_empty(), "some misses fall on ticks of their own");
    for tick in &alone {
        let (Some(before), Some(after)) = (
            value_at(&told.target_hp, tick - 1),
            value_at(&told.target_hp, *tick),
        ) else {
            continue;
        };
        assert!(
            after >= before,
            "a miss on tick {tick} took health: {before} -> {after}"
        );
    }
    // Blight Stone's armor cut is the attack's on-hit debuff, handed over after the third blow.
    // The fourth misses: the armor holds through it, and the cut shows once the fifth has landed
    // (the view after its tick).
    let blow = |n: usize| told.blows[n - 1];
    assert_eq!(
        blow(4).1,
        Blow::Missed,
        "seed 5's fourth uphill blow misses"
    );
    assert_eq!(blow(5).1, Blow::Landed, "and its fifth lands");
    let armor_before = value_at(&told.target_armor, blow(4).0 - 1).expect("seen before the miss");
    let first_cut = told
        .target_armor
        .iter()
        .find(|(_, armor)| *armor < armor_before)
        .map(|(tick, _)| *tick)
        .expect("Blight Stone cut the target's armor");
    assert!(
        first_cut > blow(4).0 && first_cut <= blow(5).0 + 1,
        "the cut came on tick {first_cut}, with the fifth blow (tick {}), not the missed fourth \
         (tick {}): {:?}",
        blow(5).0,
        blow(4).0,
        told.target_armor
    );
}

#[test]
fn a_spectator_and_the_replay_are_told_of_every_miss() {
    let played = play(&UPHILL);
    let misses = played.attacker.misses();
    assert!(misses > 0, "the attacker missed");
    // The spectator and the replay keep reading past the counted blows: at least as many.
    assert!(
        played.spectator_misses >= misses,
        "the spectator was told of {} of {misses} misses",
        played.spectator_misses
    );
    assert!(
        played.replay_misses >= misses,
        "the replay holds {} of {misses} misses",
        played.replay_misses
    );
}

#[test]
fn the_same_swings_across_level_river_ground_never_miss() {
    let played = play(&Scene {
        name: "level",
        attacker_at: RIVER,
        target_at: RIVER_TOO,
        attacker_items: &[],
        target_items: &[],
        handed_after: None,
    });
    assert_eq!(played.attacker.misses(), 0, "{:?}", played.attacker.blows);
    assert_eq!(played.target.misses(), 0);
}

#[test]
fn a_butterfly_on_the_target_makes_level_swings_miss_and_both_seats_are_told() {
    let played = play(&Scene {
        name: "butterfly",
        attacker_at: RIVER,
        target_at: RIVER_TOO,
        attacker_items: &[],
        target_items: &[ItemId::BUTTERFLY],
        handed_after: None,
    });
    // Seed 5's run of Dota's pseudo-random 35%.
    assert_eq!(
        played.attacker.misses(),
        14,
        "Butterfly misses over {BLOWS} blows: {:?}",
        played.attacker.blows
    );
    assert_eq!(played.target.misses(), played.attacker.misses());
}
