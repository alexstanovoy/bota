//! Who may watch without a seat's key, and which orders a watcher is told:
//! live ones only through the eyes its key opens.

use bota_proto::{PlayerId, SeatKey, SlotOrder, Vec2};

use super::*;
use crate::spectators::Spectators;

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
    // A seat's line is taken after its order, and an order is told with the tick it is taken on
    // or the one after: the watcher has been told every order once it is shown the tick after
    // the later line.
    let spoken: Vec<ServerMsg> = [&mut c1, &mut c2]
        .into_iter()
        .flat_map(|client| client.say_all_and_hear("go"))
        .collect();
    let said_at = spoken
        .iter()
        .filter_map(|msg| match msg {
            ServerMsg::Heard(heard) => heard.tick,
            _ => None,
        })
        .max()
        .expect("a line said in the match");
    let told = watcher.collect_through_tick(said_at + 2);
    assert!(
        told.iter()
            .filter_map(|msg| snapshot(msg.clone()))
            .all(|view| view.viewer == Some(Team::Dire)),
        "the chosen side's fog throughout"
    );
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
    // The match stands waiting for acknowledgements on `live`. A late spectator hears its own
    // lines as they are said, so a line it says comes after every tick it is shown.
    watcher.send(&say(Audience::Spectators, "now"));
    let told = watcher.collect_until(|msg| matches!(msg, ServerMsg::Heard(_)));
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

/// Players are not told about spectators: a seat hears nothing of one
/// coming or going, in the lobby or the match, and its number counts seats
/// only; spectators are told a fellow left.
#[test]
fn a_seat_is_told_nothing_of_spectators_coming_and_going() {
    let mut opts = realtime(23);
    opts.spectators = Spectators::Open;
    let addr = start_server(opts);
    let mut early = TestClient::connect(addr);
    early.hello(Role::Spectator, None);
    let welcomed = |client: &mut TestClient| {
        client.recv_until(|msg| match msg {
            ServerMsg::Welcome { player_id, .. } => Some(player_id),
            _ => None,
        })
    };
    assert!(welcomed(&mut early).0 > 2, "a watcher is numbered apart");
    early.leave();
    let mut first = TestClient::connect(addr);
    first.hello(Role::Bot, None);
    assert_eq!(welcomed(&mut first), PlayerId(1), "the first seat is one");
    // A seat's own pick is told back to it as the lobby; reading to it flushes what it was
    // told on joining, and the next pick or readiness marks the end of what a watcher's
    // coming and going could have told it.
    first.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    first.collect_until(|msg| seat_zero(msg).is_some_and(|seat| seat.hero.is_some()));
    let mut lobby_watcher = TestClient::connect(addr);
    lobby_watcher.hello(Role::Spectator, None);
    welcomed(&mut lobby_watcher);
    lobby_watcher.leave();
    first.send(&ClientMsg::SetReady(true));
    let told = first.collect_until(|msg| seat_zero(msg).is_some_and(|seat| seat.ready));
    assert_eq!(
        told.len(),
        1,
        "a seat is told nothing of a watcher: {told:?}"
    );

    let mut second = TestClient::connect(addr);
    second.hello(Role::Bot, None);
    assert_eq!(welcomed(&mut second), PlayerId(2));
    second.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    second.send(&ClientMsg::SetReady(true));
    first.recv_until(snapshot);
    second.recv_until(snapshot);
    let mut stays = TestClient::connect(addr);
    stays.hello(Role::Spectator, None);
    welcomed(&mut stays);
    let mut leaves = TestClient::connect(addr);
    leaves.hello(Role::Spectator, None);
    let gone = welcomed(&mut leaves);
    leaves.leave();
    let left = |messages: &[ServerMsg]| -> Vec<(PlayerId, Option<SlotId>)> {
        messages
            .iter()
            .filter_map(|msg| match msg {
                ServerMsg::ParticipantLeft { player_id, slot } => Some((*player_id, *slot)),
                _ => None,
            })
            .collect()
    };
    // The seat's line, said once the watcher is gone, comes after anything the departure
    // queued for the others.
    const AFTER: &str = "the watcher is gone";
    let told_first = first.say_all_and_hear(AFTER);
    let after = |msg: &ServerMsg| matches!(msg, ServerMsg::Heard(heard) if heard.line.body == Body::Text(AFTER.to_owned()));
    assert_eq!(left(&stays.collect_until(after)), [(gone, None)]);
    for told in [told_first, second.collect_until(after)] {
        assert!(left(&told).is_empty(), "{:?}", left(&told));
    }
    second.leave();
    let told = first.say_all_and_hear("and the other seat");
    assert_eq!(
        left(&told),
        [(PlayerId(2), Some(SlotId(1)))],
        "a seat's leaving is told"
    );
}

/// What a lobby state says of the first seat.
fn seat_zero(msg: &ServerMsg) -> Option<&bota_proto::LobbySlot> {
    match msg {
        ServerMsg::LobbyState { slots, .. } => slots.first(),
        _ => None,
    }
}
