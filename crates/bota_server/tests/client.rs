//! The client library's bot loop plays a lockstep match against the server
//! over a real socket, to its end: the loop of the `attack_move` example,
//! over a compressed link as a bot far from its server would ask for.

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use bota_client_core::{IssuedOrder, Link, Transport, play};
use bota_core::fountain_pos;
use bota_proto::{
    ClientMsg, Codec, HeroId, ItemId, MapId, Order, ServerMsg, SlotId, Target, Team, TickMode,
};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run};
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
        players: 2,
        replay: None,
        replay_max_bytes: None,
        meta: None,
        meta_trailer: false,
        seed: Some(3),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(2),
        cheats: false,
        immortal_structures: false,
        immortal_heroes: false,
        objectives: None,
        runes: None,
        dummy_seat: true,
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
        empty_after: None,
    }
}

#[test]
fn the_attack_move_bot_plays_a_lockstep_match_to_its_end() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || run(listener, opts()));
    let (mut link, seated) =
        Link::join(&address, "attack-move", HeroId::SHADOW_FIEND, None).expect("a seat");
    link.send(&ClientMsg::Compress(Codec::Deflate))
        .expect("the request goes out");
    // While the hero lies dead it still shops, as a Dota player does: one
    // Tango a death, into the stash.
    let (mut shopped, mut stashed) = (false, false);
    let outcome = play(&mut link, seated, &mut (), |session| {
        let enemy = fountain_pos(session.map(), session.enemy());
        let stash = session.own_player().and_then(|me| me.stash.as_ref());
        stashed |=
            stash.is_some_and(|stash| stash.iter().flatten().any(|item| item.id == ItemId::TANGO));
        let dead = session.own_hero().is_none();
        let buy = dead && !shopped;
        shopped = dead;
        [
            buy.then_some(IssuedOrder::hero(Order::Buy {
                item: ItemId::TANGO,
            })),
            Some(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(enemy),
            })),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
    })
    .expect("the match is played to its end");
    assert!(stashed, "a Tango bought while dead waits in the stash");
    assert_eq!((outcome.slot, outcome.team), (SlotId(0), Team::Radiant));
    assert!(outcome.ticks > 0);
    assert_eq!(
        outcome.rejections, 0,
        "only a live hero is sent a body's orders; the shop takes a dead one's"
    );
    assert!(
        outcome.unsent_while_dead > 0,
        "the hero fell under the tower and was held back while dead"
    );
    server
        .join()
        .expect("the server thread")
        .expect("a clean end");
}

/// Dota's LEAVE GAME: the seat that leaves a running match loses it at once, and the server names
/// the leaver. A seat that merely disconnects is waited for.
#[test]
fn a_seat_that_leaves_loses_the_match() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let mut options = opts();
    options.mode = TickMode::Realtime;
    options.dummy_seat = false;
    let server = thread::spawn(move || run(listener, options));
    let (mut stayer, seated) =
        Link::join(&address, "stays", HeroId::SHADOW_FIEND, None).expect("a seat");
    let (mut leaver, _) =
        Link::join(&address, "leaves", HeroId::SHADOW_FIEND, None).expect("a seat");
    let stays = thread::spawn(move || play(&mut stayer, seated, &mut (), |_| Vec::new()));
    // The match runs once a snapshot comes.
    while !matches!(
        leaver.recv().expect("the link holds"),
        Some(ServerMsg::Snapshot { .. })
    ) {}
    leaver.send(&ClientMsg::LeaveGame).expect("sent");
    drop(leaver);
    let end = server.join().expect("the server thread").expect("an end");
    let MatchEnd::Over { winner, left, .. } = end else {
        panic!("the match ends with a result: {end:?}");
    };
    assert_eq!((winner, left), (Team::Radiant, Some(1)));
    stays.join().expect("the stayer").expect("it saw the end");
}

/// `--tick-rate` is the wall-clock pace alone: a server paced at 120 ticks a second tells its
/// clients the game runs 30, so no clock reads half the game time.
///
/// Two servers play at once, one paced at 120 and one at 30, and each client reports the ticks it
/// is sent. However slowly a loaded machine runs them, the one paced at 120 is the first to reach
/// a tick the other needs four times as long for; and neither runs faster than its pace.
#[test]
fn a_faster_pace_is_told_as_the_games_own_tick_rate() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Instant;

    const TARGET: u32 = 240;
    struct Watched {
        tick: Arc<AtomicU32>,
        stop: Arc<AtomicBool>,
        told: thread::JoinHandle<(Option<u16>, Instant)>,
    }
    let watch = |tick_rate: u16| {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = listener.local_addr().expect("bound").to_string();
        let mut options = opts();
        options.mode = TickMode::Realtime;
        options.tick_rate = tick_rate;
        thread::spawn(move || run(listener, options));
        let (tick, stop) = (
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let (seen, halt) = (Arc::clone(&tick), Arc::clone(&stop));
        let told = thread::spawn(move || {
            let (mut link, seated) =
                Link::join(&address, "pace", HeroId::SHADOW_FIEND, None).expect("a seat");
            assert_eq!(seated.tick_rate, 30, "the welcome");
            let (mut told, mut began) = (None, Instant::now());
            while !halt.load(Ordering::SeqCst) {
                match link.recv().expect("the server speaks") {
                    Some(ServerMsg::MatchStart { info }) => {
                        told = Some(info.tick_rate);
                        began = Instant::now();
                    }
                    Some(ServerMsg::Snapshot { view }) if told.is_some() => {
                        seen.store(view.tick, Ordering::SeqCst);
                    }
                    _ => {}
                }
            }
            (told, began)
        });
        Watched { tick, stop, told }
    };
    let (fast, slow) = (watch(120), watch(30));
    let deadline = Instant::now() + Duration::from_secs(300);
    while fast.tick.load(Ordering::SeqCst) < TARGET {
        assert!(
            Instant::now() < deadline,
            "the faster server reached tick {TARGET}"
        );
        thread::sleep(Duration::from_millis(5));
    }
    let reached = Instant::now();
    let behind = slow.tick.load(Ordering::SeqCst);
    for watched in [&fast, &slow] {
        watched.stop.store(true, Ordering::SeqCst);
    }
    // The second client may be waiting for a tick that never comes now: it ends with its server.
    let (told, began) = fast.told.join().expect("the fast client");
    assert_eq!(told, Some(30), "the match start");
    // Tick 240 is two seconds of the pace asked for: never sooner.
    assert!(
        reached.duration_since(began) >= Duration::from_millis(1_900),
        "{TARGET} ticks in {:?}",
        reached.duration_since(began)
    );
    // The server paced at 30 needs eight seconds for it: it was not there when the other was.
    assert!(behind < TARGET, "the slower pace was at tick {behind}");
    drop(slow.told);
}
