//! The server speaks the protocol end to end over a real socket.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use bota_proto::{
    ClientMsg, FrameReader, HeroId, Order, RejectReason, ReplayRecord, Role, ServerMsg, Target,
    Team, Vec2, encode_frame_to_vec,
};

use crate::game_loop::{ServerOpts, run};

struct TestClient {
    stream: TcpStream,
    reader: FrameReader,
}

impl TestClient {
    fn connect(addr: std::net::SocketAddr) -> TestClient {
        let stream = TcpStream::connect(addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("set timeout");
        TestClient {
            stream,
            reader: FrameReader::new(),
        }
    }

    fn send(&mut self, msg: &ClientMsg) {
        let frame = encode_frame_to_vec(msg).expect("encode");
        self.stream.write_all(&frame).expect("send");
    }

    fn recv(&mut self) -> ServerMsg {
        let mut buf = [0u8; 65536];
        loop {
            if let Some(msg) = self.reader.next_message::<ServerMsg>().expect("decode") {
                return msg;
            }
            let n = self.stream.read(&mut buf).expect("read");
            assert!(n > 0, "server hung up while a message was awaited");
            self.reader.push(&buf[..n]);
        }
    }

    fn recv_until<T>(&mut self, mut pick: impl FnMut(ServerMsg) -> Option<T>) -> T {
        for _ in 0..300 {
            if let Some(found) = pick(self.recv()) {
                return found;
            }
        }
        panic!("the awaited message never came");
    }
}

fn start_server(opts: ServerOpts) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || run(listener, opts));
    addr
}

fn join_as_bot(addr: std::net::SocketAddr, name: &str) -> (TestClient, Option<bota_proto::SlotId>) {
    let mut client = TestClient::connect(addr);
    client.send(&ClientMsg::Hello {
        role: Role::Bot,
        name: name.to_string(),
    });
    let slot = client.recv_until(|msg| match msg {
        ServerMsg::Welcome { slot, .. } => Some(slot),
        _ => None,
    });
    client.send(&ClientMsg::PickHero { hero: HeroId(0) });
    client.send(&ClientMsg::SetReady(true));
    (client, slot)
}

#[test]
fn a_realtime_match_reaches_two_clients() {
    let replay_path = std::env::temp_dir().join("bota-net-test-realtime.brp");
    let addr = start_server(ServerOpts {
        mode: bota_proto::TickMode::Realtime,
        tick_rate: 60,
        players: 2,
        replay: Some(replay_path.clone()),
        seed: 7,
        map: bota_proto::MapId(0),
        ack_timeout_ticks: 150,
    });

    let (mut c1, slot1) = join_as_bot(addr, "alpha");
    let (mut c2, slot2) = join_as_bot(addr, "beta");
    assert_eq!(slot1, Some(bota_proto::SlotId(0)));
    assert_eq!(slot2, Some(bota_proto::SlotId(1)));

    for client in [&mut c1, &mut c2] {
        client.recv_until(|msg| match msg {
            ServerMsg::MatchStart { info } => {
                assert_eq!(info.picks.len(), 2);
                Some(())
            }
            _ => None,
        });
    }

    let first = c1.recv_until(|msg| match msg {
        ServerMsg::Snapshot { view } => Some(view),
        _ => None,
    });
    assert_eq!(first.viewer, Some(Team::Radiant));
    assert_eq!(first.players.len(), 2);
    assert!(first.units.iter().any(|u| u.team == Team::Radiant));
    let first_events = c1.recv_until(|msg| match msg {
        ServerMsg::Events { tick, events } if tick == first.tick => Some(events),
        _ => None,
    });
    assert!(first_events.is_empty());

    // A legal order passes silently; an impossible one is named and refused.
    c1.send(&ClientMsg::Order {
        seq: 1,
        unit: None,
        order: Order::Move {
            target: Target::Pos(Vec2::from_ints(1000, 1000)),
        },
    });
    c1.send(&ClientMsg::Order {
        seq: 2,
        unit: None,
        order: Order::Buy {
            item: bota_proto::ItemId(999),
        },
    });
    let (seq, reason) = c1.recv_until(|msg| match msg {
        ServerMsg::OrderRejected { seq, reason } => Some((seq, reason)),
        _ => None,
    });
    assert_eq!(seq, 2);
    assert_eq!(reason, RejectReason::UnknownItem);

    let later = c1.recv_until(|msg| match msg {
        ServerMsg::Snapshot { view } if view.tick > first.tick => Some(view),
        _ => None,
    });
    assert!(later.tick > first.tick);

    // Hanging up empties the seats; the server ends the match and flushes.
    drop(c1);
    drop(c2);
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(50));
        let bytes = std::fs::read(&replay_path).unwrap_or_default();
        if !bytes.is_empty() {
            let mut reader = FrameReader::new();
            reader.push(&bytes);
            match reader.next_message::<ReplayRecord>() {
                Ok(Some(ReplayRecord::Msg(ServerMsg::MatchStart { .. }))) => return,
                _ => continue, // still being written
            }
        }
    }
    panic!("the replay never became readable");
}

#[test]
fn a_spectator_borrows_eyes_and_is_told_the_orders_they_see() {
    let addr = start_server(ServerOpts {
        mode: bota_proto::TickMode::Realtime,
        tick_rate: 60,
        players: 2,
        replay: None,
        seed: 9,
        map: bota_proto::MapId(0),
        ack_timeout_ticks: 150,
    });
    let (mut c1, _) = join_as_bot(addr, "alpha");
    let (mut c2, _) = join_as_bot(addr, "beta");
    let mut watcher = TestClient::connect(addr);
    watcher.send(&ClientMsg::Hello {
        role: Role::Spectator,
        name: "eye".to_string(),
    });
    let slot = watcher.recv_until(|msg| match msg {
        ServerMsg::Welcome { slot, .. } => Some(slot),
        _ => None,
    });
    assert_eq!(slot, None, "a spectator takes no seat");
    let first = watcher.recv_until(|msg| match msg {
        ServerMsg::Snapshot { view } => Some(view),
        _ => None,
    });
    assert_eq!(
        first.viewer, None,
        "everything, before any eyes are asked for"
    );

    // A free view is a clean one: an order given while nobody's eyes are
    // borrowed is told to no spectator.
    c1.send(&ClientMsg::Order {
        seq: 1,
        unit: None,
        order: Order::Move {
            target: Target::Pos(Vec2::from_ints(1000, 1000)),
        },
    });
    let mut told_freely = 0u32;
    for _ in 0..3 {
        watcher.recv_until(|msg| match msg {
            ServerMsg::Orders { .. } => {
                told_freely += 1;
                None
            }
            ServerMsg::Snapshot { .. } => Some(()),
            _ => None,
        });
    }
    assert_eq!(told_freely, 0, "watching everything is told no orders");

    watcher.send(&ClientMsg::ViewAs {
        seat: Some(bota_proto::SlotId(1)),
    });
    let fogged = watcher.recv_until(|msg| match msg {
        ServerMsg::Snapshot { view } if view.viewer.is_some() => Some(view),
        _ => None,
    });
    assert_eq!(
        fogged.viewer,
        Some(Team::Dire),
        "the snapshots come through the chosen eyes"
    );

    // Through one side's eyes, only that side's orders are told.
    c2.send(&ClientMsg::Order {
        seq: 1,
        unit: None,
        order: Order::Move {
            target: Target::Pos(Vec2::from_ints(15000, 15000)),
        },
    });
    let told = watcher.recv_until(|msg| match msg {
        ServerMsg::Orders { orders, .. } => Some(orders),
        _ => None,
    });
    assert!(
        told.iter().all(|given| given.slot == bota_proto::SlotId(1)),
        "an order of the side watched: {told:?}"
    );
}

#[test]
fn lockstep_advances_as_fast_as_the_acks_come() {
    let addr = start_server(ServerOpts {
        mode: bota_proto::TickMode::Lockstep,
        tick_rate: 60,
        players: 2,
        replay: None,
        seed: 8,
        map: bota_proto::MapId(0),
        ack_timeout_ticks: 600, // ten seconds: only acks can move this match
    });

    let (mut c1, _) = join_as_bot(addr, "alpha");
    let (mut c2, _) = join_as_bot(addr, "beta");

    // Both ack every snapshot; five ticks must pass well inside the timeout.
    let mut last_tick = 0;
    for _ in 0..5 {
        for client in [&mut c1, &mut c2] {
            let tick = client.recv_until(|msg| match msg {
                ServerMsg::Snapshot { view } => Some(view.tick),
                _ => None,
            });
            client.send(&ClientMsg::Ack { tick });
            last_tick = last_tick.max(tick);
        }
    }
    assert!(last_tick >= 4, "the match advanced on acknowledgements");
}
