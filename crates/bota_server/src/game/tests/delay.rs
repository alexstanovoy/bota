//! Orders held back on their way to the simulation.

use crate::game::{Command, DelayLine, OrderDelay, RefusedOrder, World};
use bota_proto::{ItemId, Order, OrderDelayView, RejectReason, SlotId, Target, Team, Vec2};

use super::support::*;

/// An order of seat 0 to its hero, given unqueued.
fn hero_order(seq: u32, order: Order) -> Command {
    Command {
        slot: SlotId(0),
        unit: None,
        order,
        queue: false,
        seq,
    }
}

/// A walk whose goal names the order it came from.
fn walk(seq: u32) -> Order {
    Order::Move {
        target: Target::Pos(Vec2::from_ints(6000 + seq as i32, 6000)),
    }
}

/// Gives one walk a tick for `ticks` ticks; returns, for every tick, which
/// walk took effect on it and the delay the view of that tick carries.
fn play(delay: OrderDelay, ticks: u32) -> Vec<(Option<u32>, Vec<OrderDelayView>)> {
    let cfg = config();
    let mut world = World::for_match(&cfg, cfg.rng());
    let mut line = DelayLine::new(&cfg, &[delay]).expect("a delay within bounds");
    let mut refused = Vec::new();
    let mut ticked = Vec::new();
    for seq in 0..ticks {
        let seen = Some(world.tick);
        assert_eq!(line.give(&world, hero_order(seq, walk(seq)), seen), Ok(()));
        let commands = line.release(&world, &mut refused);
        assert!(refused.is_empty());
        assert!(commands.len() <= 1);
        let applied = commands.first().map(|command| {
            let Order::Move {
                target: Target::Pos(goal),
            } = command.order
            else {
                panic!("only walks were given");
            };
            (goal.x.to_int() - 6000) as u32
        });
        world.advance(&commands);
        ticked.push((applied, line.views(Some(Team::Radiant))));
    }
    ticked
}

#[test]
fn jittered_orders_never_overtake_and_wait_within_their_bounds() {
    let delay = OrderDelay { base: 2, jitter: 3 };
    let ticked = play(delay, 300);
    assert_eq!(
        ticked,
        play(delay, 300),
        "the jitter is a function of the seed"
    );
    let applied: Vec<u32> = ticked.iter().filter_map(|(seq, _)| *seq).collect();
    assert!(applied.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(
        applied.len() > 100,
        "most walks take effect: {}",
        applied.len()
    );
    let mut told = Vec::new();
    for (_, views) in &ticked {
        for view in views {
            assert_eq!(view.slot, SlotId(0));
            assert!((2..=5).contains(&view.ticks), "{view:?}");
            told.push(view.ticks);
        }
    }
    told.sort_unstable();
    told.dedup();
    assert!(told.len() > 1, "the jitter was drawn: {told:?}");
}

#[test]
fn without_a_delay_every_order_takes_effect_on_the_next_tick() {
    let ticked = play(OrderDelay::NONE, 20);
    for (seq, (applied, views)) in ticked.iter().enumerate() {
        assert_eq!(*applied, Some(seq as u32));
        assert_eq!(
            views,
            &vec![OrderDelayView {
                slot: SlotId(0),
                ticks: 0
            }]
        );
    }
}

#[test]
fn an_order_that_no_longer_holds_on_its_tick_is_refused_then() {
    let cfg = config();
    let mut world = World::for_match(&cfg, cfg.rng());
    let mut line = DelayLine::new(&cfg, &[OrderDelay { base: 1, jitter: 0 }]).expect("bounded");
    let boots = Order::Buy {
        item: ItemId::BOOTS,
    };
    let mut refused = Vec::new();
    for seq in 0..3 {
        if seq < 2 {
            let given = line.give(&world, hero_order(seq, boots), Some(world.tick));
            assert_eq!(given, Ok(()), "the gold for one pair is there when given");
        }
        let commands = line.release(&world, &mut refused);
        world.advance(&commands);
    }
    assert_eq!(
        refused,
        vec![RefusedOrder {
            slot: SlotId(0),
            seq: 1,
            reason: RejectReason::NotEnoughGold,
        }]
    );
}

/// Cheats given on one tick all go through, whatever else is given then:
/// a cheat is no order to a body, so it neither replaces nor is replaced.
#[test]
fn cheats_given_on_one_tick_all_take_effect() {
    let cfg = crate::game::MatchConfig {
        cheats: true,
        ..config()
    };
    let mut world = World::for_match(&cfg, cfg.rng());
    let mut line = DelayLine::new(&cfg, &[]).expect("no delay");
    let seen = Some(world.tick);
    for (seq, order) in [
        Order::Cheat {
            cheat: bota_proto::Cheat::StartGame,
        },
        walk(1),
        Order::Cheat {
            cheat: bota_proto::Cheat::CreepSpawning { on: false },
        },
        Order::Cheat {
            cheat: bota_proto::Cheat::Gold { amount: 100 },
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            line.give(&world, hero_order(seq as u32, order), seen),
            Ok(())
        );
    }
    let mut refused = Vec::new();
    let commands = line.release(&world, &mut refused);
    assert_eq!(commands.len(), 4, "every one goes through");
    let gold = world.seats[0].gold;
    world.advance(&commands);
    assert!(world.clock_shift.game > 0, "the game began");
    assert!(world.creeps_halted);
    assert_eq!(world.seats[0].gold, gold + 100);
}

/// Purchases given on one tick all go through, and a walk given with them
/// takes nothing away: buying is no order to a body, so it neither replaces
/// nor is replaced (a client's choices of the tactical phase all arrive
/// together when the match begins).
#[test]
fn purchases_given_on_one_tick_all_take_effect() {
    let cfg = config();
    let mut world = World::for_match(&cfg, cfg.rng());
    let mut line = DelayLine::new(&cfg, &[]).expect("no delay");
    let seen = Some(world.tick);
    let buy = |item| Order::Buy { item };
    let orders = [
        buy(ItemId::MANGO),
        buy(ItemId::MANGO),
        walk(2),
        buy(ItemId::CLARITY),
    ];
    for (seq, order) in orders.into_iter().enumerate() {
        assert_eq!(
            line.give(&world, hero_order(seq as u32, order), seen),
            Ok(())
        );
    }
    let mut refused = Vec::new();
    let commands = line.release(&world, &mut refused);
    assert_eq!(commands.len(), 4, "every one goes through");
    let gold = world.seats[0].gold;
    world.advance(&commands);
    assert!(refused.is_empty(), "{refused:?}");
    let cost = |item| bota_core::item_def(item).expect("sold").cost;
    assert_eq!(
        world.seats[0].gold,
        gold - 2 * cost(ItemId::MANGO) - cost(ItemId::CLARITY)
    );
}
