//! The Twin Gates end to end, over real sockets in a lockstep match on the Dota map: a hero
//! clicks a gate, channels four seconds while both gates glow, and comes out on the other
//! gate's exit to the hundredth of a unit, either side; a side that sees neither gate nor the
//! hero is told nothing of it.

use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use bota_client_core::{Ingested, IssuedOrder, Link, Seat, SessionOptions, Transport};
use bota_core::{DOTA_TWIN_GATES, WARP_CHANNEL_TICKS};
use bota_proto::{
    Cheat, EffectId, EventKind, HeroId, MapId, Order, StatusFlags, Target, TickMode, UnitKind,
    UnitView, Vec2, WorldView,
};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

fn opts(players: u8) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players,
        replay: None,
        replay_max_bytes: None,
        meta: None,
        meta_trailer: false,
        seed: Some(5),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        // The Dota map stands its Twin Gates by default.
        map: MapId(0),
        cheats: true,
        lobby_cheats: false,
        immortal_structures: false,
        immortal_heroes: true,
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

/// What one seat saw of the warps.
#[derive(Debug, Default)]
struct Seen {
    /// The tick its hero was first seen channelling, and the tick it was seen on the far exit.
    began: Option<u32>,
    landed: Option<u32>,
    /// Where its hero stood when it was first seen out of the channel.
    landing: Option<Vec2>,
    /// Whether it saw each gate glow while its hero channelled.
    glowed: [bool; 2],
    /// Whether any gate ever glowed in its view, or any warp was told to it.
    any_glow: bool,
    told: Vec<EventKind>,
    /// The seat's side.
    team: Option<bota_proto::Team>,
}

fn gate_index(unit: &UnitView) -> Option<usize> {
    (unit.kind == UnitKind::TwinGate)
        .then(|| DOTA_TWIN_GATES.iter().position(|gate| gate.at == unit.pos))
        .flatten()
}

fn glows(unit: &UnitView) -> bool {
    unit.effects
        .iter()
        .any(|effect| effect.id == EffectId::TWIN_GATE_ACTIVE)
}

/// One seat's play: `warp_from` names the gate its hero is put before and clicks, none for a
/// seat that stays at its fountain. It plays until `done` is set; it sets it itself once its
/// warp has landed. `joined` is told once it holds its seat.
fn play(
    address: &str,
    warp_from: Option<usize>,
    done: &AtomicBool,
    joined: mpsc::Sender<()>,
) -> Seen {
    let (mut link, seated) =
        Link::join(address, "gatekeeper", HeroId::SHADOW_FIEND, None).expect("a seat");
    let _ = joined.send(());
    let mut seat = Seat::new(seated, SessionOptions::default());
    let mut seen = Seen::default();
    let (mut stage, mut ticks) = (0u8, 0u32);
    while !done.load(Ordering::SeqCst) {
        ticks += 1;
        assert!(ticks < 20_000, "the warp never landed: {seen:?}");
        let msg = link.recv().expect("the link").expect("open");
        let Ingested::TickComplete { .. } = seat.ingest(msg, &mut ()).expect("a sound stream")
        else {
            continue;
        };
        let session = seat.session_mut().expect("started");
        seen.team = Some(session.team());
        let view: WorldView = session.current().expect("a view").clone();
        for told in session.recent_events() {
            if matches!(told.kind, EventKind::GateWarped { .. }) && !seen.told.contains(&told.kind)
            {
                seen.told.push(told.kind.clone());
            }
        }
        let gates: Vec<(usize, &UnitView)> = view
            .units
            .iter()
            .filter_map(|unit| Some((gate_index(unit)?, unit)))
            .collect();
        seen.any_glow |= gates.iter().any(|(_, gate)| glows(gate));
        let order = match (warp_from, session.own_hero()) {
            (Some(from), Some(hero)) => {
                let before = DOTA_TWIN_GATES[from].at + Vec2::from_ints(0, -170);
                let channelling = hero.statuses.bits & StatusFlags::CHANNELLING != 0;
                if channelling {
                    seen.began.get_or_insert(view.tick);
                    for (index, gate) in &gates {
                        seen.glowed[*index] |= glows(gate);
                    }
                } else if seen.began.is_some() && seen.landed.is_none() {
                    seen.landed = Some(view.tick);
                    seen.landing = Some(hero.pos);
                    done.store(true, Ordering::SeqCst);
                }
                match stage {
                    0 => Some(Order::Cheat {
                        cheat: Cheat::Teleport { at: before },
                    }),
                    1 if hero.pos.within(before, bota_proto::Fixed::from_int(50)) => gates
                        .iter()
                        .find(|(index, _)| *index == from)
                        .map(|(_, gate)| Order::Attack {
                            target: Target::Unit(gate.id),
                        }),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(order) = order {
            stage += 1;
            let outgoing = session.issue(IssuedOrder::hero(order)).expect("issued");
            link.send(&outgoing.message()).expect("sent");
        }
        if let Some(tick) = session.end_turn() {
            link.send(&bota_proto::ClientMsg::Ack { tick })
                .expect("acked");
        }
    }
    seen
}

#[test]
fn a_hero_warps_from_the_south_east_gate_to_the_north_west_exit_in_four_seconds() {
    let (address, server) = serve(opts(1));
    let done = AtomicBool::new(false);
    let seen = play(&address, Some(1), &done, mpsc::channel().0);
    drop(server);
    let (began, landed) = (
        seen.began.expect("it channelled"),
        seen.landed.expect("it landed"),
    );
    assert!(
        (WARP_CHANNEL_TICKS..=WARP_CHANNEL_TICKS + 1).contains(&(landed - began)),
        "a four-second channel: {began}..{landed}"
    );
    assert_eq!(
        seen.landing,
        Some(DOTA_TWIN_GATES[0].exit),
        "on the north-western gate's exit"
    );
    assert_eq!(
        seen.glowed,
        [false, true],
        "its own gate glows; the far one it does not see"
    );
    assert_eq!(seen.told.len(), 1, "told of its warp: {:?}", seen.told);
}

#[test]
fn a_dire_hero_warps_north_west_to_south_east_and_radiant_at_its_fountain_learns_nothing() {
    let (address, server) = serve(opts(2));
    let done = Arc::new(AtomicBool::new(false));
    let (joined, seated) = mpsc::channel();
    // The seat that joins first is Radiant's; the warper joins once it holds it.
    let watcher = {
        let (address, done) = (address.clone(), done.clone());
        thread::spawn(move || play(&address, None, &done, joined))
    };
    seated.recv().expect("the Radiant seat joined");
    let warper = {
        let (address, done) = (address.clone(), done.clone());
        thread::spawn(move || play(&address, Some(0), &done, mpsc::channel().0))
    };
    let (watched, warped) = (
        watcher.join().expect("the Radiant seat"),
        warper.join().expect("the Dire seat"),
    );
    drop(server);
    assert_eq!(watched.team, Some(bota_proto::Team::Radiant));
    assert_eq!(warped.team, Some(bota_proto::Team::Dire));
    assert_eq!(warped.landing, Some(DOTA_TWIN_GATES[1].exit));
    assert!(!watched.any_glow, "no gate glowed in Radiant's view");
    assert!(
        watched.told.is_empty(),
        "nor was a warp told: {:?}",
        watched.told
    );
}
