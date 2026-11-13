//! Team-conditioned map obstacles through real lockstep seats and real orders.
use bota_client_core::{Ingested, IssuedOrder, Link, Seat, SessionOptions, Transport};
use bota_core::{rules, segment_distance_squared};
use bota_proto::{Cheat, HeroId, MapId, Order, Target, Team, TickMode, Vec2};
use bota_server::game_loop::{DEFAULT_SPECTATORS_PER_SEAT, ServerOpts, run};
use bota_server::net::DEFAULT_MESSAGES_PER_SEC;
use bota_server::spectators::Spectators;
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

const GATES: [(Vec2, bool, Team); 4] = [
    (Vec2::from_ints(1523, 6287), false, Team::Radiant),
    (Vec2::from_ints(16576, 11648), false, Team::Dire),
    (Vec2::from_ints(5771, 1741), true, Team::Radiant),
    (Vec2::from_ints(12160, 16512), true, Team::Dire),
];

fn options() -> ServerOpts {
    ServerOpts {
        pick_time: Duration::from_secs(rules::PICK_SECS),
        pick_penalty_time: Duration::from_secs(rules::PICK_PENALTY_SECS),
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
        seed: Some(5),
        match_id: None,
        reserved: Vec::new(),
        referee_key: None,
        start_after: None,
        max_connections: None,
        map: MapId(0),
        cheats: true,
        lobby_cheats: false,
        immortal_structures: true,
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
        empty_after: Some(Duration::ZERO),
    }
}

struct Client {
    link: Link,
    seat: Seat,
}

impl Client {
    fn join(address: &str) -> Self {
        let (link, seated) =
            Link::join(address, "gate_walk", HeroId::SHADOW_FIEND, None).expect("a seat");
        Client {
            link,
            seat: Seat::new(seated, SessionOptions::default()),
        }
    }

    fn turn(&mut self, order: impl FnOnce(Team, Vec2) -> Option<Order>) {
        let mut complete = false;
        for _ in 0..500 {
            let message = self.link.recv().expect("the stream").expect("open");
            if matches!(
                self.seat.ingest(message, &mut ()).expect("sound stream"),
                Ingested::TickComplete { .. }
            ) {
                complete = true;
                break;
            }
        }
        assert!(complete, "the server did not finish a turn");
        let session = self.seat.session_mut().expect("started");
        let hero = session.own_hero().expect("hero");
        let request = order(session.team(), hero.pos);
        if let Some(order) = request {
            let outgoing = session.issue(IssuedOrder::hero(order)).expect("issued");
            self.link.send(&outgoing.message()).expect("sent");
        }
        if let Some(tick) = session.end_turn() {
            self.link
                .send(&bota_proto::ClientMsg::Ack { tick })
                .expect("acked");
        }
    }
}

fn check_field(centre: Vec2, vertical: bool, previous: Vec2, next: Vec2) {
    for index in -7..=7 {
        let offset = if vertical {
            Vec2::from_ints(0, index * 25)
        } else {
            Vec2::from_ints(index * 25, 0)
        };
        assert!(
            segment_distance_squared(centre + offset, previous, next)
                >= rules::units(rules::HERO_COLLISION + 24).squared_raw(),
            "enemy crossed field{centre:?}: {previous:?} -> {next:?}"
        );
    }
}

fn walk_case(clients: &mut [Client; 2], gate: (Vec2, bool, Team), team: Team) {
    for client in clients.iter_mut() {
        client.turn(|side, _| {
            Some(Order::Cheat {
                cheat: Cheat::Teleport {
                    at: bota_core::hero_spawn_pos(bota_core::map_of(MapId(0)), side),
                },
            })
        });
    }
    let (centre, vertical, owner) = gate;
    let across = if vertical {
        Vec2::from_ints(70, 0)
    } else {
        Vec2::from_ints(0, 70)
    };
    let (start, finish) = (centre - across, centre + across);
    let (mut stage, mut steps, mut previous, mut reached) = (0u8, 0u32, start, false);
    for _ in 0..185 {
        for client in clients.iter_mut() {
            client.turn(|side, at| {
                if side != team {
                    return None;
                }
                match stage {
                    0 => {
                        stage = 1;
                        Some(Order::Cheat {
                            cheat: Cheat::Teleport { at: start },
                        })
                    }
                    1 => {
                        assert!(
                            at.within(start, rules::units(2)),
                            "cheat did not land at{start:?}: {at:?}"
                        );
                        stage = 2;
                        previous = at;
                        Some(Order::Move {
                            target: Target::Pos(finish),
                        })
                    }
                    _ => {
                        if team != owner {
                            check_field(centre, vertical, previous, at);
                        }
                        previous = at;
                        steps += 1;
                        reached |= at.within(finish, rules::units(10));
                        None
                    }
                }
            });
        }
        if steps >= 180 {
            break;
        }
    }
    assert!(steps >= 180, "the walk must run its bounded scenario");
    if team == owner {
        assert!(reached, "allied{team:?} never walked through{centre:?}");
    }
}

#[test]
fn real_seats_walk_allied_base_exits_and_never_walk_through_enemy_fields() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = thread::spawn(move || run(listener, options()));
    let mut clients = [Client::join(&address), Client::join(&address)];
    for index in 0..2 {
        for client in &mut clients {
            client.turn(|side, _| {
                (index == 0 && side == Team::Radiant).then_some(Order::Cheat {
                    cheat: Cheat::StartGame,
                })
            });
        }
    }
    for gate in GATES {
        for team in [Team::Radiant, Team::Dire] {
            walk_case(&mut clients, gate, team);
        }
    }
    drop(clients);
    assert!(server.join().expect("server thread").is_ok());
}
