//! Dota's cheats end to end: a client types each command (in its chat or
//! console form, through the client library's parser) into a lockstep match
//! over a real socket, and sees what it did in the views that come back; a
//! match without cheats refuses every one; the replay records that cheats
//! were on and every cheat used; and a match of reserved seats (what the
//! matchmaking service launches) cannot be started with them.

use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use bota_client_core::{
    CommandContext, Ingested, IssuedOrder, Link, Plain, Seat, SessionOptions, Transport,
    UnpackedReplay, parse_command,
};
use bota_proto::{
    Cheat, FrameReader, HeroId, ItemId, MapId, Order, RejectReason, ReplayRecord, ServerMsg,
    SlotId, StatusFlags, TickMode, UnitKind, Vec2, WorldView,
};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, check_opts, run};
use bota_server::lobby::ReservedSeat;
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

fn opts(cheats: bool, replay: Option<PathBuf>) -> ServerOpts {
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
        seed: Some(5),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(1),
        cheats,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: false,
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
        empty_after: Some(Duration::from_secs(1)),
    }
}

/// The client's names, with the cursor where the test points.
struct Pointing(Vec2);

impl CommandContext for Pointing {
    fn item(&self, word: &str) -> Option<ItemId> {
        Plain.item(word)
    }

    fn hero(&self, word: &str) -> Option<HeroId> {
        Plain.hero(word)
    }

    fn cursor(&self) -> Option<Vec2> {
        Some(self.0)
    }
}

/// One step: what is typed (or, for a Demo Hero button, the cheat itself),
/// and what the next views must show.
struct Step {
    asked: Asked,
    holds: fn(&WorldView) -> bool,
}

enum Asked {
    Typed(&'static str),
    Button(Cheat),
}

fn me(view: &WorldView) -> &bota_proto::PlayerView {
    view.player(SlotId(0)).expect("own row")
}

fn my_hero(view: &WorldView) -> Option<&bota_proto::UnitView> {
    let id = me(view).unit?;
    view.units.iter().find(|unit| unit.id == id)
}

fn creeps(view: &WorldView) -> usize {
    view.units
        .iter()
        .filter(|unit| {
            matches!(
                unit.kind,
                UnitKind::CreepMelee
                    | UnitKind::CreepFlagbearer
                    | UnitKind::CreepRanged
                    | UnitKind::CreepSiege
            )
        })
        .count()
}

/// Whether the client's hero carries an item.
fn carries(view: &WorldView, item: ItemId) -> bool {
    my_hero(view).is_some_and(|hero| hero.items.iter().flatten().any(|held| held.id == item))
}

/// Where the test points: open ground a little way from the Radiant
/// fountain of the demo map.
fn spot() -> Vec2 {
    let map = bota_core::map_of(MapId(1));
    let fountain = bota_core::hero_spawn_pos(map, bota_proto::Team::Radiant);
    fountain + Vec2::from_ints(1200, 1200)
}

fn steps() -> Vec<Step> {
    use Asked::{Button, Typed};
    vec![
        Step {
            asked: Typed("-gold 5000"),
            holds: |v| me(v).gold.is_some_and(|gold| gold >= 5000),
        },
        Step {
            asked: Typed("dota_dev hero_level 3"),
            holds: |v| me(v).level == 4,
        },
        // Items by their Dota keys, in the chat's and the console's form.
        Step {
            asked: Typed("-item item_blink"),
            holds: |v| carries(v, ItemId::BLINK_DAGGER),
        },
        Step {
            asked: Typed("dota_create_item item_flask"),
            holds: |v| carries(v, ItemId::HEALING_SALVE),
        },
        Step {
            asked: Typed("-startgame"),
            holds: |v| v.game_tick() >= bota_core::rules::PREGAME_TICKS,
        },
        Step {
            asked: Typed("-createhero pudge enemy"),
            holds: |v| {
                v.player(SlotId(1))
                    .is_some_and(|p| p.hero == HeroId::PUDGE && p.team == bota_proto::Team::Dire)
            },
        },
        Step {
            asked: Typed("-levelbots 2"),
            holds: |v| v.player(SlotId(1)).is_some_and(|p| p.level == 3),
        },
        Step {
            asked: Typed("-allvision"),
            holds: |v| {
                let pudge = v.player(SlotId(1)).and_then(|p| p.unit);
                pudge.is_some_and(|id| v.units.iter().any(|unit| unit.id == id))
            },
        },
        Step {
            asked: Typed("-spawncreeps"),
            holds: |v| creeps(v) > 0,
        },
        Step {
            asked: Typed("-killcreeps"),
            holds: |v| creeps(v) == 0,
        },
        Step {
            asked: Typed("-disablecreepspawn"),
            holds: |_| true,
        },
        Step {
            asked: Typed("-enablecreepspawn"),
            holds: |_| true,
        },
        Step {
            asked: Typed("-spawnneutrals"),
            holds: |_| true,
        },
        Step {
            asked: Typed("-wtf"),
            holds: |v| {
                my_hero(v)
                    .is_some_and(|hero| hero.abilities.iter().all(|ability| ability.mana_cost == 0))
            },
        },
        Step {
            asked: Typed("-unwtf"),
            holds: |v| {
                my_hero(v)
                    .is_some_and(|hero| hero.abilities.iter().any(|ability| ability.mana_cost > 0))
            },
        },
        Step {
            asked: Typed("-refresh"),
            holds: |v| my_hero(v).is_some_and(|hero| hero.hp == hero.max_hp),
        },
        Step {
            asked: Typed("-teleport"),
            holds: |v| {
                my_hero(v)
                    .is_some_and(|hero| hero.pos.within(spot(), bota_proto::Fixed::from_int(400)))
            },
        },
        Step {
            asked: Typed("-respawn"),
            holds: |v| {
                let map = bota_core::map_of(MapId(1));
                let home = bota_core::hero_spawn_pos(map, bota_proto::Team::Radiant);
                my_hero(v)
                    .is_some_and(|hero| hero.pos.within(home, bota_proto::Fixed::from_int(400)))
            },
        },
        Step {
            asked: Typed("-invulnerable"),
            holds: |v| {
                my_hero(v).is_some_and(|hero| hero.statuses.bits & StatusFlags::INVULNERABLE != 0)
            },
        },
        Step {
            asked: Button(Cheat::TimeOfDay { night: true }),
            holds: |v| v.clock.is_some_and(|clock| clock.day > 0),
        },
        Step {
            asked: Button(Cheat::CreateCreeps {
                enemy: true,
                at: spot(),
            }),
            holds: |v| creeps(v) > 0,
        },
        Step {
            asked: Button(Cheat::ResetHero),
            holds: |v| me(v).level == 1 && me(v).gold == Some(bota_core::rules::STARTING_GOLD),
        },
    ]
}

/// The cheat a step asks for.
fn cheat_of(asked: &Asked) -> Cheat {
    match asked {
        Asked::Typed(line) => parse_command(line, &Pointing(spot())).expect("a command"),
        Asked::Button(cheat) => *cheat,
    }
}

/// Plays the steps one after another, each once the last one's effect
/// showed (or, without cheats, once its refusal came). Returns the
/// refusals heard.
fn play_steps(address: &str) -> Vec<RejectReason> {
    let (mut link, seated) =
        Link::join(address, "cheater", HeroId::SHADOW_FIEND, None).expect("a seat");
    let mut seat = Seat::new(seated, SessionOptions::default());
    let steps = steps();
    let (mut at, mut waiting) = (0usize, false);
    let mut refused = Vec::new();
    let mut ticks = 0u32;
    while at < steps.len() {
        ticks += 1;
        assert!(ticks < 20_000, "step {at} never showed");
        let msg = link.recv().expect("the link").expect("open");
        match seat.ingest(msg, &mut ()).expect("a sound stream") {
            Ingested::Rejected { reason, .. } => {
                refused.push(reason);
                at += 1;
                waiting = false;
            }
            Ingested::TickComplete { .. } => {
                let session = seat.session_mut().expect("started");
                let view = session.current().expect("a view").clone();
                if waiting && (steps[at].holds)(&view) {
                    at += 1;
                    waiting = false;
                }
                if !waiting && at < steps.len() {
                    let order = Order::Cheat {
                        cheat: cheat_of(&steps[at].asked),
                    };
                    let outgoing = session.issue(IssuedOrder::hero(order)).expect("issued");
                    link.send(&outgoing.message()).expect("sent");
                    waiting = true;
                }
                if let Some(tick) = session.end_turn() {
                    link.send(&bota_proto::ClientMsg::Ack { tick })
                        .expect("acked");
                }
            }
            Ingested::Over { .. } => panic!("the match ended at step {at}"),
            _ => {}
        }
    }
    refused
}

fn serve(opts: ServerOpts) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || {
        let _ = run(listener, opts);
    });
    (address, server)
}

/// The records of the replay of a match whose server has ended, header first.
fn replay_records(path: &std::path::Path) -> Vec<ReplayRecord> {
    let file = std::fs::File::open(path).expect("the replay exists");
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut UnpackedReplay::new(file), &mut bytes)
        .expect("the replay is finished");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    std::iter::from_fn(|| reader.next_message::<ReplayRecord>().expect("a record")).collect()
}

#[test]
fn every_command_does_what_dotas_does_and_the_replay_keeps_them() {
    let path = std::env::temp_dir().join(format!("bota-cheats-{}.brp", std::process::id()));
    let (address, server) = serve(opts(true, Some(path.clone())));
    let refused = play_steps(&address);
    assert!(refused.is_empty(), "nothing refused: {refused:?}");
    server
        .join()
        .expect("the server ends once the seat is empty");
    let records = replay_records(&path);
    let _ = std::fs::remove_file(&path);
    let started = records.iter().find_map(|record| match record {
        ReplayRecord::Msg(ServerMsg::MatchStart { info }) => Some(info.cheats),
        _ => None,
    });
    assert_eq!(started, Some(true), "the replay knows cheats were on");
    let cheats: Vec<Cheat> = records
        .iter()
        .flat_map(|record| match record {
            ReplayRecord::Orders { orders, .. } => orders.clone(),
            _ => Vec::new(),
        })
        .filter_map(|order| match order.order {
            Order::Cheat { cheat } => Some(cheat),
            _ => None,
        })
        .collect();
    let asked: Vec<Cheat> = steps().iter().map(|step| cheat_of(&step.asked)).collect();
    assert_eq!(cheats, asked, "every cheat used, in order");
}

#[test]
fn a_match_without_cheats_refuses_every_command() {
    let (address, server) = serve(opts(false, None));
    let refused = play_steps(&address);
    assert_eq!(refused.len(), steps().len());
    assert!(
        refused
            .iter()
            .all(|reason| *reason == RejectReason::NoCheats),
        "{refused:?}"
    );
    server
        .join()
        .expect("the server ends once the seat is empty");
}

#[test]
fn a_match_of_reserved_seats_cannot_have_cheats() {
    let mut reserved = opts(true, None);
    reserved.players = 2;
    reserved.reserved = (0..2)
        .map(|seat| ReservedSeat {
            key: bota_proto::SeatKey([seat; 32]),
            name: format!("p{seat}"),
            team: if seat == 0 {
                bota_proto::Team::Radiant
            } else {
                bota_proto::Team::Dire
            },
            kind: None,
        })
        .collect();
    assert!(
        check_opts(&reserved).is_err(),
        "matchmaking's matches stay clean"
    );
    reserved.cheats = false;
    assert!(check_opts(&reserved).is_ok());
    assert!(
        check_opts(&opts(true, None)).is_ok(),
        "a local match may cheat"
    );
}
