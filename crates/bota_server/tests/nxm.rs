//! Matches of any size on the full map, end to end over real sockets: any lineup of reserved
//! seats, lopsided or with one side empty, joins, picks and plays with waves on all three lanes
//! until one side leaves; and five heroes pushing all three lanes against five break them tower
//! by tower, turn their waves super and then mega, and win by the Ancient, told to every seat
//! and kept in the replay.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread;
use std::time::Duration;

use bota_client_core::{ClientSession, IssuedOrder, Link, Outcome, Transport, UnpackedReplay};
use bota_core::{creep_spawn_pos, map_of, rules};
use bota_proto::{
    Cheat, ClientMsg, EntityId, EventKind, FrameReader, HeroId, MapId, Order, ReplayRecord,
    SeatKey, ServerMsg, Target, Team, TickMode, UnitKind, Vec2,
};
use bota_server::game_loop::{
    DEFAULT_IDLE_TIMEOUT, DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run,
};
use bota_server::lobby::ReservedSeat;
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

const FULL_MAP: MapId = MapId(0);

type Decide = Box<dyn FnMut(&ClientSession) -> Vec<IssuedOrder> + Send>;

fn opts(players: u8, reserved: Vec<ReservedSeat>, replay: Option<PathBuf>) -> ServerOpts {
    let cheats = reserved.is_empty();
    ServerOpts {
        pick_time: Duration::from_secs(rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players,
        replay,
        seed: Some(11),
        match_id: None,
        reserved,
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: FULL_MAP,
        cheats,
        immortal_structures: false,
        immortal_heroes: cheats,
        objectives: None,
        runes: None,
        dummy_seat: false,
        ack_timeout_ticks: 900,
        idle_timeout: Some(DEFAULT_IDLE_TIMEOUT),
        order_delays: Vec::new(),
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: Some(Duration::from_secs(5)),
    }
}

fn serve(opts: ServerOpts) -> (String, thread::JoinHandle<std::io::Result<MatchEnd>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    (address, thread::spawn(move || run(listener, opts)))
}

/// A link that leaves the match for good once it has acknowledged `leave_at`.
struct Leaving {
    link: Link,
    leave_at: Option<u32>,
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
            self.link.send(&ClientMsg::LeaveGame)?;
        }
        Ok(())
    }
}

/// Own lane creeps by the lane whose spawn they stand nearest, a second after the first wave.
fn own_creeps_by_lane(session: &ClientSession) -> [usize; 3] {
    let mut lanes = [0; 3];
    let Some(view) = session.current() else {
        return lanes;
    };
    let map = map_of(FULL_MAP);
    for unit in view.units.iter().filter(|u| {
        u.team == session.team() && matches!(u.kind, UnitKind::CreepMelee | UnitKind::CreepRanged)
    }) {
        let lane = (0..3u8)
            .min_by_key(|lane| {
                (creep_spawn_pos(map, session.team(), *lane) - unit.pos).length_squared()
            })
            .expect("three lanes");
        lanes[usize::from(lane)] += 1;
    }
    lanes
}

/// Plays a lineup of reserved seats, Radiant and Dire interleaved in slot order: every seat
/// joins with its key, picks, and sees its first waves on all three lanes; then the side that
/// leaves (Dire unless it has nobody) leaves for good and the other wins.
fn play_lineup(radiant: u8, dire: u8) {
    let teams: Vec<Team> = (0..radiant.max(dire))
        .flat_map(|i| {
            [
                (i < radiant).then_some(Team::Radiant),
                (i < dire).then_some(Team::Dire),
            ]
        })
        .flatten()
        .collect();
    let keys: Vec<SeatKey> = (0..teams.len())
        .map(|i| SeatKey([i as u8 + 1; 32]))
        .collect();
    let reserved = teams
        .iter()
        .zip(&keys)
        .enumerate()
        .map(|(i, (team, key))| ReservedSeat {
            key: *key,
            name: format!("seat{i}"),
            team: *team,
            kind: None,
        })
        .collect();
    let (address, server) = serve(opts(0, reserved, None));
    let leaver = if dire > 0 { Team::Dire } else { Team::Radiant };
    let seen_at = rules::FIRST_WAVE_TICK + rules::TICKS_PER_SECOND;
    let seats: Vec<_> = teams
        .iter()
        .zip(keys)
        .enumerate()
        .map(|(i, (team, key))| {
            let (address, team) = (address.clone(), *team);
            let hero = [HeroId::SHADOW_FIEND, HeroId::PUDGE][i % 2];
            thread::spawn(move || {
                let (link, seated) =
                    Link::join(&address, "bot", hero, Some(key)).expect("the reserved seat");
                let leave_at = (team == leaver).then_some(seen_at + 10);
                let mut transport = Leaving { link, leave_at };
                let mut lanes = [0; 3];
                let outcome = bota_client_core::play(&mut transport, seated, &mut (), |session| {
                    if session.current().is_some_and(|view| view.tick == seen_at) {
                        lanes = own_creeps_by_lane(session);
                        let players = session.current().map_or(0, |view| view.players.len());
                        assert_eq!(players, usize::from(radiant + dire), "every seat plays");
                    }
                    None
                })
                .expect("the match is played to its end");
                (team, outcome, lanes)
            })
        })
        .collect();
    for seat in seats {
        let (team, outcome, lanes): (Team, Outcome, [usize; 3]) =
            seat.join().expect("the seat plays");
        assert_eq!(outcome.team, team, "{radiant}v{dire}: the seat's own side");
        assert_eq!(
            outcome.winner,
            leaver.enemy(),
            "{radiant}v{dire}: {outcome:?}"
        );
        assert!(
            lanes.iter().all(|n| *n >= 4),
            "{radiant}v{dire}: waves {lanes:?}"
        );
    }
    match server.join().expect("the server").expect("it ran") {
        MatchEnd::Over { winner, left, .. } => {
            assert_eq!(winner, leaver.enemy(), "{radiant}v{dire}");
            assert!(left.is_some(), "{radiant}v{dire}: given away by a leaver");
        }
        MatchEnd::Abandoned => panic!("{radiant}v{dire}: abandoned"),
    }
}

#[test]
fn any_lineup_takes_its_reserved_seats_and_plays_all_three_lanes() {
    for (radiant, dire) in [
        (1, 1),
        (2, 2),
        (3, 3),
        (5, 5),
        (1, 2),
        (2, 5),
        (1, 0),
        (0, 3),
    ] {
        play_lineup(radiant, dire);
    }
}

/// The enemy structures one lane's pusher takes in turn: its towers, then its barracks.
fn lane_targets(lane: u8) -> Vec<Vec2> {
    let towers = rules::DIRE_TOWERS
        .iter()
        .filter(|(l, tier, _)| *l == lane && *tier <= 3)
        .map(|(_, tier, at)| (*tier, *at));
    let mut towers: Vec<(u8, Vec2)> = towers.collect();
    towers.sort_by_key(|(tier, _)| *tier);
    let barracks = rules::DIRE_BARRACKS
        .iter()
        .filter(|(l, _, _)| *l == lane)
        .map(|(_, _, at)| *at);
    towers
        .into_iter()
        .map(|(_, at)| at)
        .chain(barracks)
        .collect()
}

/// The way to the Ancient once every lane is broken: the tier fours, then the Ancient.
fn throne_targets() -> Vec<Vec2> {
    let t4 = rules::DIRE_TOWERS.iter().filter(|(_, tier, _)| *tier == 4);
    t4.map(|(_, _, at)| *at)
        .chain([rules::DIRE_ANCIENT_POS])
        .collect()
}

/// Whether a target is down: the hero stands near enough to see it and no enemy building
/// stands on it.
fn fallen(session: &ClientSession, target: Vec2) -> bool {
    let Some(view) = session.current() else {
        return false;
    };
    let near = |a: Vec2, b: Vec2, r: i32| {
        (a - b).length_squared() <= Vec2::from_ints(r, 0).length_squared()
    };
    let Some(hero) = session.own_hero() else {
        return false;
    };
    near(hero.pos, target, 900)
        && !view.units.iter().any(|u| {
            u.team == Team::Dire
                && matches!(
                    u.kind,
                    UnitKind::Tower | UnitKind::Barracks | UnitKind::Ancient
                )
                && near(u.pos, target, 300)
        })
}

/// One Radiant pusher: levels, then breaks its lane; once all five have, waits for a mega wave
/// and takes the tier fours and the Ancient.
fn pusher(lane: u8, broken: Arc<AtomicU8>) -> impl FnMut(&ClientSession) -> Vec<IssuedOrder> {
    let mut decisions = 0u32;
    let mut targets = lane_targets(lane);
    let mut lane_done_at: Option<u32> = None;
    move |session| {
        decisions += 1;
        let hero = IssuedOrder::hero;
        match decisions {
            1 => {
                return vec![hero(Order::Cheat {
                    cheat: Cheat::Levels { count: 29 },
                })];
            }
            2 => {
                return vec![hero(Order::Cheat {
                    cheat: Cheat::StartGame,
                })];
            }
            _ => {}
        }
        let tick = session.current().map_or(0, |view| view.tick);
        while targets.first().is_some_and(|at| fallen(session, *at)) {
            targets.remove(0);
        }
        if targets.is_empty() && lane_done_at.is_none() {
            lane_done_at = Some(tick);
            broken.fetch_add(1, Ordering::SeqCst);
            targets = throne_targets();
        }
        let all_broken = broken.load(Ordering::SeqCst) == 5;
        let hold = lane_done_at.is_some() && !all_broken;
        match targets.first() {
            Some(at) if !hold => vec![hero(Order::Attack {
                target: Target::Pos(*at),
            })],
            _ => Vec::new(),
        }
    }
}

/// What the replay shows: the order Dire's structures fell in, by where they stood, the
/// biggest Radiant lane creep seen after each count of fallen Dire barracks, and the winner.
struct Replayed {
    fell: Vec<Vec2>,
    melee_hp_by_fallen_barracks: HashMap<usize, i32>,
    winner: Option<Team>,
}

fn replayed(path: &std::path::Path) -> Replayed {
    let file = std::fs::File::open(path).expect("the replay");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut UnpackedReplay::new(file), &mut bytes).expect("unpacked");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    let mut at: HashMap<EntityId, (UnitKind, Vec2)> = HashMap::new();
    let mut out = Replayed {
        fell: Vec::new(),
        melee_hp_by_fallen_barracks: HashMap::new(),
        winner: None,
    };
    while let Some(record) = reader.next_message::<ReplayRecord>().expect("a record") {
        match record {
            ReplayRecord::Msg(ServerMsg::Snapshot { view, .. }) => {
                let rax_down = out
                    .fell
                    .iter()
                    .filter(|p| rules::DIRE_BARRACKS.iter().any(|(_, _, b)| b == *p))
                    .count();
                for unit in &view.units {
                    at.insert(unit.id, (unit.kind, unit.pos));
                    if unit.team == Team::Radiant && unit.kind == UnitKind::CreepMelee {
                        let best = out.melee_hp_by_fallen_barracks.entry(rax_down).or_default();
                        *best = (*best).max(unit.max_hp);
                    }
                }
            }
            ReplayRecord::Msg(ServerMsg::Events { events, .. }) => {
                for event in events {
                    if let EventKind::StructureDestroyed {
                        unit,
                        team: Team::Dire,
                    } = event
                        && let Some((_, pos)) = at.get(&unit)
                    {
                        out.fell.push(*pos);
                    }
                }
            }
            ReplayRecord::Msg(ServerMsg::MatchOver { winner, .. }) => out.winner = Some(winner),
            _ => {}
        }
    }
    out
}

#[test]
fn five_pushing_three_lanes_break_them_in_dotas_order_and_win_by_the_ancient() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let replay = dir.join("nxm-5v5.brp");
    let (address, server) = serve(opts(10, Vec::new(), Some(replay.clone())));
    let broken = Arc::new(AtomicU8::new(0));
    // Even slots are Radiant: two push top, one mid, two bottom; Dire's five stand at home.
    let lanes = [
        rules::LANE_TOP,
        rules::LANE_MID,
        rules::LANE_BOT,
        rules::LANE_TOP,
        rules::LANE_BOT,
    ];
    let seats: Vec<_> = (0..10)
        .map(|_| {
            let (address, broken) = (address.clone(), broken.clone());
            thread::spawn(move || {
                let (mut link, seated) =
                    Link::join(&address, "bot", HeroId::SHADOW_FIEND, None).expect("a seat");
                let mut decide: Decide = if seated.slot.0.is_multiple_of(2) {
                    Box::new(pusher(lanes[usize::from(seated.slot.0 / 2)], broken))
                } else {
                    Box::new(|_: &ClientSession| Vec::new())
                };
                bota_client_core::play(&mut link, seated, &mut (), &mut *decide)
                    .expect("the match is played to its end")
            })
        })
        .collect();
    for seat in seats {
        let outcome = seat.join().expect("the seat plays");
        assert_eq!(outcome.winner, Team::Radiant, "{outcome:?}");
    }
    match server.join().expect("the server").expect("it ran") {
        MatchEnd::Over { winner, left, .. } => {
            assert_eq!((winner, left), (Team::Radiant, None));
        }
        MatchEnd::Abandoned => panic!("abandoned"),
    }
    let seen = replayed(&replay);
    assert_eq!(
        seen.winner,
        Some(Team::Radiant),
        "the replay ends with the result"
    );
    let order_of = |at: Vec2| seen.fell.iter().position(|p| *p == at);
    for lane in [rules::LANE_TOP, rules::LANE_MID, rules::LANE_BOT] {
        let fell: Vec<Option<usize>> = lane_targets(lane).into_iter().map(order_of).collect();
        assert!(
            fell.iter().all(Option::is_some),
            "lane {lane}: all fell: {fell:?}"
        );
        assert!(
            fell[0] < fell[1] && fell[1] < fell[2],
            "lane {lane}: T1, T2, T3: {fell:?}"
        );
        assert!(
            fell[2] < fell[3] && fell[2] < fell[4],
            "lane {lane}: T3, then barracks"
        );
    }
    let last = order_of(rules::DIRE_ANCIENT_POS).expect("the Ancient fell");
    assert_eq!(
        last,
        seen.fell.len() - 1,
        "the Ancient falls last: {:?}",
        seen.fell
    );
    let hp = &seen.melee_hp_by_fallen_barracks;
    assert!(hp[&0] < rules::SUPER_MELEE_HP, "plain waves before: {hp:?}");
    assert!(
        hp[&6] >= rules::MEGA_MELEE_HP,
        "mega waves after all six: {hp:?}"
    );
    assert!(
        (2..6).any(|n| hp.get(&n).is_some_and(|hp| *hp >= rules::SUPER_MELEE_HP)),
        "super waves after a lane's barracks: {hp:?}"
    );
}
