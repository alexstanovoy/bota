//! Who may watch without a seat's key, and which orders a watcher is told:
//! live ones only through the eyes its key opens.

use std::io::ErrorKind;

use bota_proto::{SeatKey, SlotOrder, Vec2};

use super::*;
use crate::spectators::Spectators;

/// Everything a client is sent for `wait`, pings answered.
fn drain(client: &mut TestClient, wait: Duration) -> Vec<ServerMsg> {
    let deadline = Instant::now() + wait;
    let mut messages = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        while let Some(msg) = client.reader.next_message::<ServerMsg>().expect("decode") {
            match msg {
                ServerMsg::Ping => client.send(&ClientMsg::Pong),
                msg => messages.push(msg),
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return messages;
        }
        client
            .stream
            .set_read_timeout(Some(left))
            .expect("set timeout");
        match client.stream.read(&mut buf) {
            Ok(0) => return messages,
            Ok(n) => client.reader.push(&buf[..n]),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => panic!("read: {error}"),
        }
    }
}

fn orders_told(messages: &[ServerMsg]) -> Vec<SlotOrder> {
    messages
        .iter()
        .filter_map(|msg| match msg {
            ServerMsg::Orders { orders, .. } => Some(orders.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn a_keyless_spectator_is_refused_unless_the_server_lets_anyone_watch() {
    let addr = start_server(realtime(20));
    for key in [None, Some(SeatKey([5; 32]))] {
        let mut watcher = TestClient::connect(addr);
        watcher.hello(Role::Spectator, key);
        assert_eq!(watcher.refusal(), RejectReason::NotAdmitted, "{key:?}");
    }
    let (_seated, slot) = join_as_bot(addr, "alpha");
    assert_eq!(slot, Some(SlotId(0)), "the seats are untouched");
}

#[test]
fn an_open_spectator_borrows_a_sides_fog_but_is_told_no_order() {
    let mut opts = realtime(9);
    opts.spectators = Spectators::Open;
    let addr = start_server(opts);
    let (mut c1, _) = join_as_bot(addr, "alpha");
    let (mut c2, _) = join_as_bot(addr, "beta");
    let mut watcher = TestClient::connect(addr);
    watcher.hello(Role::Spectator, None);
    assert_eq!(watcher.welcome(), None, "a spectator takes no seat");
    let first = watcher.recv_until(snapshot);
    assert_eq!(
        first.viewer, None,
        "everything, before any eyes are asked for"
    );

    watcher.send(&ClientMsg::ViewAs {
        seat: Some(SlotId(1)),
    });
    let fogged = watcher.recv_until(|msg| snapshot(msg).filter(|view| view.viewer.is_some()));
    assert_eq!(fogged.viewer, Some(Team::Dire), "the chosen side's fog");
    for (seq, client) in [&mut c1, &mut c2].into_iter().enumerate() {
        client.send(&move_order(seq as u32 + 1, 15000, 15000));
    }
    let told = drain(&mut watcher, Duration::from_millis(500));
    assert!(told.iter().filter_map(|msg| snapshot(msg.clone())).count() > 5);
    assert_eq!(
        orders_told(&told),
        Vec::new(),
        "borrowed eyes are told no order"
    );
}

#[test]
fn the_key_that_claimed_a_seat_watches_it_live_with_its_orders() {
    let addr = start_server(realtime(21));
    let key = SeatKey([7; 32]);
    let mut c1 = TestClient::connect(addr);
    c1.hello(Role::Bot, Some(key));
    assert_eq!(c1.welcome(), Some(SlotId(0)));
    c1.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    c1.send(&ClientMsg::SetReady(true));
    let mut watcher = TestClient::connect(addr);
    watcher.hello(Role::Spectator, Some(key));
    assert_eq!(watcher.welcome(), None);
    let (_c2, _) = join_as_bot(addr, "beta");
    // Its eyes are its key's: asking for the other side's changes nothing.
    watcher.send(&ClientMsg::ViewAs {
        seat: Some(SlotId(1)),
    });
    assert_eq!(watcher.recv_until(snapshot).viewer, Some(Team::Radiant));
    // The seat names a unit as its side does, and so is it told on.
    let seen = c1.recv_until(snapshot);
    let target = Target::Unit(seen.units.last().expect("a unit of its own").id);
    c1.send(&ClientMsg::Order {
        seq: 1,
        unit: None,
        order: Order::Move { target },
        seen: None,
        queue: false,
    });
    let told = watcher.recv_until(|msg| match msg {
        ServerMsg::Orders { orders, .. } => Some(orders),
        _ => None,
    });
    assert!(told.iter().all(|given| given.slot == SlotId(0)), "{told:?}");
    assert_eq!(told[0].order, Order::Move { target });
}

/// Plays a lockstep match by hand: both seats acknowledge every tick up to
/// `last`, and the first seat gives one order on tick 5. Returns the tick the
/// match then stands on, waiting.
fn play_to(c1: &mut TestClient, c2: &mut TestClient, last: u32) -> u32 {
    let mut tick = 0;
    while tick < last {
        for client in [&mut *c1, &mut *c2] {
            tick = client.recv_until(snapshot).tick;
            client.send(&ClientMsg::Ack { tick });
        }
        if tick == 5 {
            c1.send(&move_order(1, 1000, 1000));
        }
    }
    c1.recv_until(snapshot).tick
}

#[test]
fn a_late_spectator_is_shown_the_match_that_many_seconds_behind_with_both_seats_orders() {
    let mut opts = realtime(22);
    opts.mode = bota_proto::TickMode::Lockstep;
    // A hundred seconds: only acknowledgements move this match.
    opts.ack_timeout_ticks = 6000;
    opts.spectators = Spectators::Delayed { secs: 1 };
    let delay = u32::from(opts.tick_rate);
    let addr = start_server(opts);
    let (mut c1, _) = join_as_bot(addr, "alpha");
    let (mut c2, _) = join_as_bot(addr, "beta");
    let mut watcher = TestClient::connect(addr);
    watcher.hello(Role::Spectator, None);
    assert_eq!(watcher.welcome(), None);

    let live = play_to(&mut c1, &mut c2, delay + 10);
    let told = drain(&mut watcher, Duration::from_millis(500));
    let ticks: Vec<u32> = told
        .iter()
        .filter_map(|msg| snapshot(msg.clone()))
        .map(|view| view.tick)
        .collect();
    assert_eq!(
        ticks.last(),
        Some(&(live - delay)),
        "{ticks:?} with {live} live"
    );
    assert!(
        told.iter()
            .filter_map(|msg| snapshot(msg.clone()))
            .all(|v| v.viewer.is_none())
    );
    let orders = orders_told(&told);
    assert_eq!(orders.len(), 1, "the order, a second late: {orders:?}");
    assert_eq!(orders[0].slot, SlotId(0));
    assert_eq!(
        orders[0].order,
        Order::Move {
            target: Target::Pos(Vec2::from_ints(1000, 1000))
        }
    );
}
