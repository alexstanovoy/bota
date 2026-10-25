//! The full Dota map, as a Practice game is played on it: waves on all three lanes, from both
//! sides, and the neutral camps stocked, with one player alone on it.

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use bota_client_core::{Ingested, IssuedOrder, Link, Seat, SessionOptions, Transport};
use bota_proto::{Cheat, HeroId, MapId, Order, TickMode, UnitKind, Vec2};
use bota_server::game_loop::{DEFAULT_IDLE_TIMEOUT, DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

fn opts() -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 1,
        replay: None,
        seed: Some(5),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(0),
        cheats: true,
        immortal_structures: false,
        immortal_heroes: true,
        objectives: None,
        runes: None,
        dummy_seat: false,
        ack_timeout_ticks: 150,
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

/// The distance of two points, squared, in whole units.
fn near(a: Vec2, b: Vec2) -> i64 {
    let (dx, dy) = (
        i64::from((a.x - b.x).to_int()),
        i64::from((a.y - b.y).to_int()),
    );
    dx * dx + dy * dy
}

#[test]
fn a_practice_match_has_waves_on_all_three_lanes_and_stocked_camps() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let mut options = opts();
    // The whole map to look at it with, fog or not.
    options.spectators = Spectators::Open;
    let server = thread::spawn(move || run(listener, options));
    let map = bota_core::map_of(MapId(0));

    // The one player skips the horn and keeps acknowledging ticks, which a lockstep match waits for.
    let seat_address = address.clone();
    let player = thread::spawn(move || {
        let (mut link, seated) =
            Link::join(&seat_address, "practice", HeroId::SHADOW_FIEND, None).expect("a seat");
        let mut seat = Seat::new(seated, SessionOptions::default());
        let mut started = false;
        while let Ok(Some(msg)) = link.recv() {
            let Ok(Ingested::TickComplete { .. }) = seat.ingest(msg, &mut ()) else {
                continue;
            };
            let Some(session) = seat.session_mut() else {
                continue;
            };
            if !started {
                started = true;
                let order = Order::Cheat {
                    cheat: Cheat::StartGame,
                };
                if let Ok(outgoing) = session.issue(IssuedOrder::hero(order)) {
                    let _ = link.send(&outgoing.message());
                }
            }
            if let Some(tick) = session.end_turn() {
                let _ = link.send(&bota_proto::ClientMsg::Ack { tick });
            }
        }
    });

    let mut watcher = Link::connect(&address, Duration::from_secs(30)).expect("a connection");
    watcher
        .send(&bota_proto::ClientMsg::hello(
            bota_proto::Role::Spectator,
            "looker",
            None,
            bota_core::WIRE_FINGERPRINT,
        ))
        .expect("hello");
    let mut lanes = [0usize; 3];
    let mut neutrals = 0usize;
    while let Ok(Some(msg)) = watcher.recv() {
        let bota_proto::ServerMsg::Snapshot { view } = msg else {
            continue;
        };
        // Half a minute after the horn the first wave still stands near its spawn on every
        // lane, from both sides; a minute and a half in, the camps are stocked.
        let game = view.tick.saturating_sub(bota_core::rules::PREGAME_TICKS);
        if game >= 25 * 30 && lanes == [0; 3] {
            for unit in &view.units {
                if matches!(
                    unit.kind,
                    UnitKind::CreepMelee
                        | UnitKind::CreepFlagbearer
                        | UnitKind::CreepRanged
                        | UnitKind::CreepSiege
                ) {
                    for (lane, count) in lanes.iter_mut().enumerate() {
                        if map
                            .creep_spawns
                            .iter()
                            .any(|team| near(unit.pos, team[lane]) < 2500 * 2500)
                        {
                            *count += 1;
                        }
                    }
                }
            }
        }
        if game >= 90 * 30 {
            neutrals = view
                .units
                .iter()
                .filter(|unit| unit.kind == UnitKind::CreepNeutral)
                .count();
            break;
        }
    }
    drop(watcher);
    let _ = (player.join(), server.join());
    assert!(
        lanes.iter().all(|count| *count > 0),
        "waves on top, mid and bottom: {lanes:?}"
    );
    assert!(neutrals > 0, "the camps are stocked");
}
