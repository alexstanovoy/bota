//! A departed seat cannot continue issuing orders through its existing connection.

use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use bota_client_core::{IssuedOrder, Link, Transport, UnpackedReplay, join, play};
use bota_core::fountain_pos;
use bota_proto::{
    ClientMsg, FrameReader, HeroId, MapId, Order, ReplayRecord, SeatKey, ServerMsg, SlotId, Target,
    Team, TickMode, Vec2,
};
use bota_server::game::OrderDelay;
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, MatchEnd, ServerOpts, run};
use bota_server::lobby::ReservedSeat;
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;

fn seat(key: u8, name: &str, team: Team) -> ReservedSeat {
    ReservedSeat {
        key: SeatKey([key; 32]),
        name: name.to_owned(),
        team,
        kind: None,
    }
}

fn opts(reserved: Vec<ReservedSeat>, replay: PathBuf) -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(bota_core::rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(bota_core::rules::PICK_PENALTY_SECS),
        tactical_time: Duration::ZERO,
        same_heroes: true,
        chat: bota_server::chat::ChatOpts::DEFAULT,
        mode: TickMode::Lockstep,
        tick_rate: 30,
        players: 3,
        replay: Some(replay),
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
        idle_timeout: None,
        order_delays: vec![OrderDelay { base: 1, jitter: 0 }; 3],
        messages_per_sec: DEFAULT_MESSAGES_PER_SEC,
        max_spectators_per_seat: DEFAULT_SPECTATORS_PER_SEAT,
        spectators: Spectators::Off,
        heroes: None,
        abandon_unjoined: false,
        empty_after: Some(Duration::from_secs(5)),
    }
}

/// Acknowledges every tick. At `leave_at` it sends LeaveGame; then, three ticks apart, it sends an
/// order of its own for its hero, as a client that kept playing would.
struct LeaveThenOrder {
    link: Link,
    leave_at: u32,
    left_at: Option<u32>,
    injected: u32,
}

impl Transport for LeaveThenOrder {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.link.recv()
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        self.link.send(msg)?;
        let ClientMsg::Ack { tick } = msg else {
            return Ok(());
        };
        let left_at = self.left_at;
        match left_at {
            None if *tick >= self.leave_at => {
                self.left_at = Some(*tick);
                self.link.send(&ClientMsg::LeaveGame)
            }
            Some(left) if self.injected < 3 && *tick >= left + 3 * (self.injected + 1) => {
                self.injected += 1;
                self.link.send(&ClientMsg::Order {
                    seq: 1000 + self.injected,
                    unit: None,
                    order: Order::Attack {
                        target: Target::Pos(Vec2::from_ints(7500, 7500)),
                    },
                    seen: Some(*tick),
                    queue: false,
                })
            }
            _ => Ok(()),
        }
    }
}

#[test]
fn a_seat_that_left_for_good_has_no_orders_applied_after_its_departure() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("permanent_leave_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch directory");
    let replay = dir.join(format!("orders-{}.brp", std::process::id()));
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let reserved = vec![
        seat(1, "host", Team::Radiant),
        seat(2, "friend", Team::Radiant),
        seat(3, "enemy", Team::Dire),
    ];
    let server = thread::spawn({
        let replay = replay.clone();
        move || run(listener, opts(reserved, replay))
    });
    let host = {
        let address = address.clone();
        thread::spawn(move || {
            let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
            let seated = join(
                &mut link,
                "host",
                HeroId::SHADOW_FIEND,
                Some(SeatKey([1; 32])),
            )
            .expect("a seat");
            let mut transport = LeaveThenOrder {
                link,
                leave_at: 60,
                left_at: None,
                injected: 0,
            };
            // The scripted decisions keep ordering the hero too, so the replay holds the seat's
            // orders from before its departure.
            let _ = play(&mut transport, seated, &mut (), |session| {
                let enemy = fountain_pos(session.map(), session.enemy());
                Some(IssuedOrder::hero(Order::Attack {
                    target: Target::Pos(enemy),
                }))
            });
            transport.injected
        })
    };
    let friend = {
        let address = address.clone();
        thread::spawn(move || {
            let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
            let seated = join(
                &mut link,
                "friend",
                HeroId::SHADOW_FIEND,
                Some(SeatKey([2; 32])),
            )
            .expect("a seat");
            play(&mut link, seated, &mut (), |_| Vec::new()).expect("played");
        })
    };
    let enemy = {
        let address = address.clone();
        thread::spawn(move || {
            let mut link = Link::connect(&address, Duration::from_secs(60)).expect("connected");
            let seated = join(
                &mut link,
                "enemy",
                HeroId::SHADOW_FIEND,
                Some(SeatKey([3; 32])),
            )
            .expect("a seat");
            let mut leaving = LeaveAndHangUp {
                link,
                leave_at: 240,
            };
            let _ = play(&mut leaving, seated, &mut (), |_| Vec::new());
        })
    };
    let injected = host.join().expect("the host thread");
    friend.join().expect("the friend thread");
    enemy.join().expect("the enemy thread");
    let end = server.join().expect("the server").expect("it ran");
    let MatchEnd::Over { report, .. } = end else {
        panic!("the match ends with a result: {end:?}");
    };
    let left = report
        .stats
        .leaves
        .iter()
        .find(|left| left.slot == SlotId(0))
        .map(|left| left.tick)
        .expect("the host's departure is reported");
    assert_eq!(
        injected, 3,
        "precondition: the seat sent its three orders after it left"
    );

    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut UnpackedReplay::new(std::fs::File::open(&replay).expect("replay")),
        &mut bytes,
    )
    .expect("the replay unpacks");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    let (mut before, mut after): (u32, Vec<u32>) = (0, Vec::new());
    let mut raw: Vec<u32> = Vec::new();
    while let Some(record) = reader
        .next_message::<ReplayRecord>()
        .expect("a whole record")
    {
        if let ReplayRecord::Orders { tick, orders } = record {
            for order in orders.iter().filter(|order| order.slot == SlotId(0)) {
                if tick <= left {
                    before += 1;
                } else {
                    after.push(tick);
                    if let Order::Attack {
                        target: Target::Pos(at),
                    } = &order.order
                        && *at == Vec2::from_ints(7500, 7500)
                    {
                        raw.push(tick);
                    }
                }
            }
        }
    }
    assert!(
        before > 0,
        "precondition: the hero took orders before it left at tick {left}"
    );
    assert!(
        after.is_empty(),
        "the seat left for good at tick {left}, yet {} of its orders were applied afterwards, first at tick {}; {} of them are the raw orders it sent after LeaveGame, first applied at tick {}",
        after.len(),
        after.first().copied().unwrap_or(0),
        raw.len(),
        raw.first().copied().unwrap_or(0)
    );
}

/// Acknowledges every tick; once `leave_at` is acknowledged it sends LeaveGame and hangs up.
struct LeaveAndHangUp {
    link: Link,
    leave_at: u32,
}

impl Transport for LeaveAndHangUp {
    fn recv(&mut self) -> std::io::Result<Option<ServerMsg>> {
        self.link.recv()
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        self.link.send(msg)?;
        match msg {
            ClientMsg::Ack { tick } if *tick >= self.leave_at => {
                self.link.send(&ClientMsg::LeaveGame)?;
                Err(std::io::Error::other("hung up after leaving"))
            }
            _ => Ok(()),
        }
    }
}
