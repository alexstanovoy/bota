//! Every unit of a seat takes orders of its own, and takes up what it was
//! given queued one order after another, as Dota's Shift-queue does.

use bota_proto::{
    AbilityId, AbilitySlot, EventKind, Order, OutgoingOrder, RejectReason, ServerMsg, Target,
    UnitView, Vec2,
};

use super::local::mid_config;
use crate::game::{Entity, OrderDelay, UnitOrder};
use crate::local::LocalMatch;

/// Ticks a drill may take before it is called failed.
const LONGEST: u32 = 600;

/// A match of two fiends whose orders take effect with `delay`, and the
/// radiant seat's hero.
fn drill(delay: OrderDelay) -> (LocalMatch, Entity) {
    let (local, _) = LocalMatch::new(&mid_config(), &[delay]).expect("a match");
    let hero = local.world().seats[0].unit.expect("the hero stands");
    (local, hero)
}

/// An order of the radiant seat, given on the view of the tick the match
/// stands on.
fn given(local: &LocalMatch, seq: u32, unit: Option<bota_proto::EntityId>) -> OutgoingOrder {
    OutgoingOrder {
        seq,
        unit,
        order: Order::Move {
            target: Target::None,
        },
        seen: local.tick(),
        queue: false,
    }
}

/// A walk of the radiant hero to `to`, queued or not.
fn walk(local: &LocalMatch, seq: u32, to: Vec2, queue: bool) -> OutgoingOrder {
    OutgoingOrder {
        order: Order::Move {
            target: Target::Pos(to),
        },
        queue,
        ..given(local, seq, None)
    }
}

/// The radiant hero as its own side last saw it in a step's messages.
fn own_hero(local: &LocalMatch, hero: Entity, stream: &[ServerMsg]) -> UnitView {
    let name = local.handle_of(0, hero).expect("the side knows its hero");
    stream
        .iter()
        .rev()
        .find_map(|msg| match msg {
            ServerMsg::Snapshot { view } => view.unit(name).cloned(),
            _ => None,
        })
        .expect("a snapshot with the hero")
}

/// Spots round the radiant hero's start, on the fountain's open ground.
fn spots(local: &LocalMatch, hero: Entity) -> [Vec2; 4] {
    let at = local.world().transform.get(hero).expect("standing").pos;
    [
        at + Vec2::from_ints(300, 0),
        at + Vec2::from_ints(300, 300),
        at + Vec2::from_ints(0, 300),
        at + Vec2::from_ints(-200, 200),
    ]
}

#[test]
fn walks_given_queued_are_walked_in_turn_and_an_unqueued_order_empties_the_queue() {
    let (mut local, hero) = drill(OrderDelay::NONE);
    let [a, b, c, d] = spots(&local, hero);
    // One burst: the first walk takes the body, the next two queue behind it.
    let burst = [
        walk(&local, 1, a, false),
        walk(&local, 2, b, true),
        walk(&local, 3, c, true),
    ];
    let step = local.step(&[&burst, &[]]).expect("a step");
    let queued = own_hero(&local, hero, &step[0]).queue;
    let told = |to| Order::Move {
        target: Target::Pos(to),
    };
    assert_eq!(queued, vec![told(b), told(c)], "its side is told the queue");
    let mut reached = Vec::new();
    for _ in 0..LONGEST {
        let step = local.step(&[&[], &[]]).expect("a step");
        let seen = own_hero(&local, hero, &step[0]);
        for (index, spot) in [a, b, c].into_iter().enumerate() {
            if seen.pos == spot && !reached.contains(&index) {
                reached.push(index);
            }
        }
        if reached.len() == 3 {
            assert!(seen.queue.is_empty(), "the queue is walked empty");
            break;
        }
    }
    assert_eq!(reached, vec![0, 1, 2], "each spot in turn");

    // Queued then unqueued on one tick: the unqueued one replaces both.
    let burst = [walk(&local, 4, a, true), walk(&local, 5, d, false)];
    let step = local.step(&[&burst, &[]]).expect("a step");
    assert!(own_hero(&local, hero, &step[0]).queue.is_empty());
    let orders = local.world().orders.get(hero).expect("orders").current;
    assert_eq!(orders, UnitOrder::Move { pos: d });
    // A queue given behind a walk, then an unqueued order a tick later.
    let burst = [walk(&local, 6, a, false), walk(&local, 7, b, true)];
    local.step(&[&burst, &[]]).expect("a step");
    let stop = [given(&local, 8, None)];
    let step = local.step(&[&stop, &[]]).expect("a step");
    assert!(own_hero(&local, hero, &step[0]).queue.is_empty());
    for _ in 0..LONGEST / 4 {
        local.step(&[&[], &[]]).expect("a step");
    }
    let orders = local.world().orders.get(hero).expect("orders").current;
    assert_eq!(
        orders,
        UnitOrder::Stand,
        "nothing queued came up after the stop"
    );
}

#[test]
fn a_queued_cast_waits_for_the_walk_and_one_that_no_longer_holds_is_refused() {
    let (mut local, hero) = drill(OrderDelay::NONE);
    let [a, ..] = spots(&local, hero);
    let near = AbilitySlot(0);
    let learn = OutgoingOrder {
        order: Order::Learn { slot: near },
        ..given(&local, 1, None)
    };
    // An ability with no point in it is refused queued as unqueued: learnt first.
    local.step(&[&[learn], &[]]).expect("a step");
    let raze = |seq| OutgoingOrder {
        order: Order::Cast {
            slot: near,
            target: Target::None,
        },
        queue: true,
        ..given(&local, seq, None)
    };
    let burst = [walk(&local, 2, a, false), raze(3), raze(4)];
    let mut arrived = None;
    let mut cast = None;
    let mut refused = Vec::new();
    let mut step = local.step(&[&burst, &[]]).expect("a step");
    for _ in 0..LONGEST {
        let tick = local.tick();
        if arrived.is_none() && own_hero(&local, hero, &step[0]).pos == a {
            arrived = Some(tick);
        }
        for msg in &step[0] {
            match msg {
                ServerMsg::OrderRejected { seq, reason } => refused.push((*seq, *reason, tick)),
                ServerMsg::Events { events, .. } => {
                    let razed = events.iter().any(|event| {
                        matches!(event, EventKind::AbilityCast { ability, .. }
                            if *ability == AbilityId::RAZE_NEAR)
                    });
                    if razed {
                        assert_eq!(cast, None, "one raze, the second is on cooldown");
                        cast = Some(tick);
                    }
                }
                _ => {}
            }
        }
        if !refused.is_empty() {
            break;
        }
        step = local.step(&[&[], &[]]).expect("a step");
    }
    let (arrived, cast) = (arrived.expect("it walked"), cast.expect("it razed"));
    assert!(arrived <= cast, "the raze waited for the walk");
    let [(seq, reason, when)] = refused[..] else {
        panic!("one refusal, of the second raze: {refused:?}");
    };
    assert_eq!((seq, reason), (4, RejectReason::OnCooldown));
    assert!(
        cast < when,
        "refused when it came up, after the first went off"
    );
}

#[test]
fn the_hero_and_the_courier_take_their_orders_given_on_one_tick_together() {
    let (mut local, hero) = drill(OrderDelay { base: 2, jitter: 8 });
    let courier = local.world().seats[0].courier.expect("a courier");
    let courier_name = local.handle_of(0, courier).expect("the side knows it");
    let [a, _, c, _] = spots(&local, hero);
    let to_courier = OutgoingOrder {
        order: Order::Move {
            target: Target::Pos(c),
        },
        ..given(&local, 2, Some(courier_name))
    };
    let burst = [walk(&local, 1, a, false), to_courier];
    local.step(&[&burst, &[]]).expect("a step");
    let current =
        |local: &LocalMatch, unit| local.world().orders.get(unit).expect("orders").current;
    let mut took = [None, None];
    for _ in 0..LONGEST {
        local.step(&[&[], &[]]).expect("a step");
        let tick = local.tick();
        for (at, (unit, to)) in [(hero, a), (courier, c)].into_iter().enumerate() {
            if took[at].is_none() && current(&local, unit) == (UnitOrder::Move { pos: to }) {
                took[at] = Some(tick);
            }
        }
        if took.iter().all(Option::is_some) {
            break;
        }
    }
    let [Some(hero_took), Some(courier_took)] = took else {
        panic!("both orders took effect: {took:?}");
    };
    assert_eq!(hero_took, courier_took, "one burst, one jitter");
}
