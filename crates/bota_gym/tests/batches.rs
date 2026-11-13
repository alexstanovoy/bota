//! Several orders from one seat in one turn: what a client may send between two ticks.

mod support;

use bota_client_core::{AckPolicy, ClientSession, IssuedOrder, MatchStatic, SessionOptions};
use bota_gym::{
    Episode, Game, GameId, GymError, MAX_ORDERS_PER_TICK, MAX_SEATS, PlayerEnv, PlayerEnvConfig,
    Privileged, SeatBatches,
};
use bota_proto::{
    AbilitySlot, ItemId, Order, OutgoingOrder, RejectReason, ServerMsg, SlotId, Target, TickMode,
    Vec2,
};
use bota_server::game::{MatchConfig, OrderDelay};
use bota_server::local::LocalMatch;

use support::{duel, picks};

/// A seat's session, built from its opening stream as a connected client builds it.
fn session(seat: usize, opening: &[ServerMsg]) -> ClientSession {
    let ServerMsg::MatchStart { info } = &opening[0] else {
        panic!("an opening begins with MatchStart");
    };
    let statics = MatchStatic::new(info).expect("statics");
    let options = SessionOptions {
        ack: AckPolicy::Never,
        ..SessionOptions::default()
    };
    let slot = SlotId(u8::try_from(seat).expect("a seat"));
    let mut session = ClientSession::seat(statics, slot, None, options).expect("a seat");
    for message in &opening[1..] {
        session.ingest(message.clone(), &mut ()).expect("folds");
    }
    session
}

/// One turn of seat 0: stop, learn its first ability, buy two cheap items, attack-move.
fn turn() -> [Order; 5] {
    [
        Order::Move {
            target: Target::None,
        },
        Order::Learn {
            slot: AbilitySlot(0),
        },
        Order::Buy {
            item: ItemId::TANGO,
        },
        Order::Buy {
            item: ItemId::CLARITY,
        },
        Order::Attack {
            target: Target::Pos(Vec2::from_ints(9_000, 9_000)),
        },
    ]
}

/// The match the gym builds from `duel(seed, …)`, set up by hand the way a server's network loop
/// takes a connection's orders: each in arrival order, through the same order line.
fn reference(seed: u64) -> (LocalMatch, Vec<Vec<ServerMsg>>) {
    let bytes = seed.to_le_bytes();
    let spec = duel(seed, 60);
    let config = MatchConfig {
        match_id: seed,
        master_key: core::array::from_fn(|index| bytes[index % bytes.len()]),
        stream: seed,
        picks: picks(),
        map: spec.map,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 150,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
        starting_gold_lost: Vec::new(),
        objectives: spec.objectives,
        runes: spec.runes,
        catalogue: spec.catalogue,
        economy_events: Vec::new(),
    };
    LocalMatch::new(&config, &[OrderDelay::NONE]).expect("a reference match")
}

#[test]
fn a_seats_batch_is_what_the_server_makes_of_the_same_orders_in_one_turn() {
    let (mut game, opening) = Game::<Privileged>::new(&duel(23, 60)).expect("a game");
    let mut client = session(0, &opening[0]);
    let batch: Vec<OutgoingOrder> = turn()
        .into_iter()
        .map(|order| client.issue(IssuedOrder::hero(order)).expect("issued"))
        .collect();
    let streams = game.step_batches(&[&batch, &[]]).expect("a tick");

    let (mut local, _) = reference(23);
    let told = local.step(&[&batch, &[]]).expect("a tick");
    assert_eq!(streams, told, "every seat is told the same");
    assert_eq!(
        format!("{:?}", game.truth().full_view()),
        format!("{:?}", local.world().view_full()),
        "the world is the same"
    );
    // And the turn did what a client meant: every order of it took effect.
    for message in streams[0].clone() {
        client.ingest(message, &mut ()).expect("folds");
    }
    let hero = client.own_hero().expect("a hero");
    let held: Vec<ItemId> = hero.items.iter().flatten().map(|item| item.id).collect();
    assert!(
        held.contains(&ItemId::TANGO) && held.contains(&ItemId::CLARITY),
        "{held:?}"
    );
    assert!(
        !streams[0]
            .iter()
            .any(|message| matches!(message, ServerMsg::OrderRejected { .. })),
        "nothing refused: {:?}",
        streams[0]
    );
}

#[test]
fn a_batch_over_the_servers_bound_is_refused_whole_before_anything_is_given() {
    let (mut game, opening) = Game::<Privileged>::new(&duel(5, 60)).expect("a game");
    let mut client = session(0, &opening[0]);
    let walk = |client: &mut ClientSession| {
        client
            .issue(IssuedOrder::hero(Order::Move {
                target: Target::Pos(Vec2::from_ints(8_000, 8_000)),
            }))
            .expect("issued")
    };
    let too_many: Vec<OutgoingOrder> = (0..=MAX_ORDERS_PER_TICK)
        .map(|_| walk(&mut client))
        .collect();
    let before = format!("{:?}", game.truth().full_view());
    let tick = game.tick();
    assert_eq!(
        game.step_batches(&[&too_many, &[]]),
        Err(GymError::OrderBatch {
            seat: 0,
            len: MAX_ORDERS_PER_TICK + 1
        })
    );
    assert_eq!(
        GymError::OrderBatch { seat: 0, len: 65 }.to_string(),
        "seat 0 gave 65 orders for one tick; at most 64 take effect"
    );
    assert_eq!(game.tick(), tick, "no tick passed");
    assert_eq!(
        format!("{:?}", game.truth().full_view()),
        before,
        "nothing was given"
    );
    // The bound itself is accepted.
    let streams = game
        .step_batches(&[&too_many[..MAX_ORDERS_PER_TICK], &[]])
        .expect("a full batch");
    assert!(
        !streams[0]
            .iter()
            .any(|message| matches!(message, ServerMsg::OrderRejected { .. })),
        "{:?}",
        streams[0]
    );
}

#[test]
fn in_a_mixed_batch_only_the_refused_order_is_answered_and_the_others_take_effect() {
    let (mut game, opening) = Game::<Privileged>::new(&duel(7, 60)).expect("a game");
    let mut client = session(0, &opening[0]);
    let gold = client
        .own_player()
        .and_then(|player| player.gold)
        .expect("own gold");
    let orders = [
        Order::Buy {
            item: ItemId::TANGO,
        },
        // A Demon Edge costs far more than a hero starts with.
        Order::Buy {
            item: ItemId::DEMON_EDGE,
        },
        Order::Learn {
            slot: AbilitySlot(0),
        },
    ];
    let batch: Vec<OutgoingOrder> = orders
        .into_iter()
        .map(|order| client.issue(IssuedOrder::hero(order)).expect("issued"))
        .collect();
    let streams = game.step_batches(&[&batch, &[]]).expect("a tick");
    let rejected: Vec<(u32, RejectReason)> = streams[0]
        .iter()
        .filter_map(|message| match message {
            ServerMsg::OrderRejected { seq, reason } => Some((*seq, *reason)),
            _ => None,
        })
        .collect();
    assert_eq!(rejected, [(batch[1].seq, RejectReason::NotEnoughGold)]);
    for message in streams[0].clone() {
        client.ingest(message, &mut ()).expect("folds");
    }
    let hero = client.own_hero().expect("a hero");
    assert!(
        hero.items
            .iter()
            .flatten()
            .any(|item| item.id == ItemId::TANGO)
    );
    assert!(client.own_player().and_then(|player| player.gold) < Some(gold));
    assert!(
        hero.abilities
            .first()
            .is_some_and(|ability| ability.level == 1),
        "the ability after the refused purchase was learned"
    );
}

#[test]
fn a_player_env_issues_several_orders_per_seat_per_turn() {
    let config = PlayerEnvConfig {
        decision_ticks: 3,
        spec: Box::new(|_, _: &()| duel(9, 60)),
    };
    let mut env = PlayerEnv::<(), bota_gym::Player, SeatBatches<4>>::start(
        &config,
        GameId { slot: 0, game: 0 },
        (),
    )
    .expect("a game");
    let mut action = SeatBatches::<4>::new();
    for seat in 0..2 {
        for order in [
            Order::Learn {
                slot: AbilitySlot(0),
            },
            Order::Buy {
                item: ItemId::TANGO,
            },
            Order::Buy {
                item: ItemId::CLARITY,
            },
        ] {
            action.push(seat, IssuedOrder::hero(order)).expect("room");
        }
    }
    assert!(env.step(&config, action).expect("a step").is_none());
    for seat in 0..2 {
        let hero = env.seat(seat).own_hero().expect("a hero");
        let held: Vec<ItemId> = hero.items.iter().flatten().map(|item| item.id).collect();
        assert!(
            held.contains(&ItemId::TANGO) && held.contains(&ItemId::CLARITY),
            "{held:?}"
        );
        assert!(
            hero.abilities
                .first()
                .is_some_and(|ability| ability.level == 1)
        );
    }
    // A full batch refuses one more.
    let mut full = SeatBatches::<1>::new();
    full.push(
        0,
        IssuedOrder::hero(Order::Learn {
            slot: AbilitySlot(0),
        }),
    )
    .expect("room");
    assert_eq!(
        full.push(
            0,
            IssuedOrder::hero(Order::Learn {
                slot: AbilitySlot(0),
            })
        ),
        Err(GymError::OrderBatch { seat: 0, len: 2 })
    );
    assert!(
        full.push(MAX_SEATS, IssuedOrder::hero(Order::Glyph))
            .is_err()
    );
}

/// Buying a Tango then selling the first bag slot leaves no Tango; in the other order the sale
/// finds the slot empty and is refused, and the Tango stays.
fn buy_then_sell() -> [Order; 2] {
    [
        Order::Buy {
            item: ItemId::TANGO,
        },
        Order::Sell {
            slot: bota_proto::ItemSlot(0),
        },
    ]
}

fn holds_tango(session: &ClientSession) -> bool {
    session
        .own_hero()
        .expect("a hero")
        .items
        .iter()
        .flatten()
        .any(|item| item.id == ItemId::TANGO)
}

#[test]
fn a_game_applies_a_seats_batch_in_the_order_given() {
    for (orders, kept) in [
        (buy_then_sell(), false),
        (
            {
                let [buy, sell] = buy_then_sell();
                [sell, buy]
            },
            true,
        ),
    ] {
        let (mut game, opening) = Game::<Privileged>::new(&duel(13, 60)).expect("a game");
        let mut client = session(0, &opening[0]);
        let batch: Vec<OutgoingOrder> = orders
            .into_iter()
            .map(|order| client.issue(IssuedOrder::hero(order)).expect("issued"))
            .collect();
        let streams = game.step_batches(&[&batch, &[]]).expect("a tick");
        for message in streams[0].clone() {
            client.ingest(message, &mut ()).expect("folds");
        }
        assert_eq!(holds_tango(&client), kept, "{orders:?}");
    }
}

#[test]
fn a_player_env_gives_a_seats_orders_in_the_order_pushed() {
    let config = PlayerEnvConfig {
        decision_ticks: 1,
        spec: Box::new(|_, _: &()| duel(13, 60)),
    };
    let mut env = PlayerEnv::<(), bota_gym::Player, SeatBatches<2>>::start(
        &config,
        GameId { slot: 0, game: 0 },
        (),
    )
    .expect("a game");
    let mut action = SeatBatches::<2>::new();
    for order in buy_then_sell() {
        action.push(0, IssuedOrder::hero(order)).expect("room");
    }
    assert!(env.step(&config, action).expect("a step").is_none());
    assert!(
        !holds_tango(env.seat(0)),
        "the second order sold what the first bought"
    );
}
