//! Dependent inventory transactions use the inventory left by earlier orders
//! in the same tick, through the server's delayed-order and side-view path.
use bota_core::map_of;
use bota_proto::{
    Cheat, HeroId, ItemId, ItemSlot, MapId, Order, OutgoingOrder, Pick, ServerMsg, SlotId, Team,
    TickMode, WorldView,
};
use bota_server::{
    game::{MatchConfig, OrderDelay},
    local::LocalMatch,
};

fn configuration() -> MatchConfig {
    let map = map_of(MapId(2));
    MatchConfig {
        match_id: 1,
        master_key: [3; 32],
        stream: 1,
        picks: vec![Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId::SHADOW_FIEND,
        }],
        map: map.id,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 150,
        cheats: true,
        spawn_modifiers: Vec::new(),
        immortal_structures: true,
        immortal_heroes: true,
        starting_gold_lost: Vec::new(),
        objectives: map.default_objectives,
        runes: map.default_runes,
        catalogue: bota_core::Catalogue::FULL,
        economy_events: Vec::new(),
    }
}

fn game() -> LocalMatch {
    let cfg = configuration();
    LocalMatch::new(&cfg, &[OrderDelay::NONE])
        .expect("a real match")
        .0
}

#[test]
fn a_second_glyph_in_one_tick_is_refused_against_the_first_glyphs_cooldown() {
    // Glyph has one shared cooldown; another request cannot cast it again:
    // https://liquipedia.net/dota2/Buildings#Glyph_of_Fortification
    let mut game = game();
    let tick = game.tick();
    game.world_mut().glyphs[0].ready_at = tick;
    let messages = step(&mut game, &[Order::Glyph, Order::Glyph]);
    let refused: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            ServerMsg::OrderRejected { seq, reason } => Some((*seq, *reason)),
            _ => None,
        })
        .collect();
    assert_eq!(
        refused,
        vec![(tick * 32 + 1, bota_proto::RejectReason::OnCooldown)]
    );
    assert!(game.world().glyphs[0].ready_at > tick);
}

fn queue_moves(game: &mut LocalMatch, count: usize) -> Vec<ServerMsg> {
    let tick = game.tick();
    let orders: Vec<_> = (0..count)
        .map(|index| OutgoingOrder {
            seq: tick * 128 + index as u32,
            unit: None,
            order: Order::Move {
                target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(8000, 8000)),
            },
            seen: tick,
            queue: true,
        })
        .collect();
    game.step(&[&orders]).expect("one server tick").remove(0)
}

#[test]
fn same_tick_queued_orders_fill_the_remaining_space_then_report_queue_full() {
    // Valve explicitly added full-queue and invalid-at-execution errors:
    // https://liquipedia.net/dota2/2011-10-27_Patch
    let mut game = game();
    step(
        &mut game,
        &[Order::Attack {
            target: bota_proto::Target::None,
        }],
    );
    let mut remaining = bota_core::rules::MAX_QUEUED_ORDERS - 1;
    while remaining > 0 {
        let count = remaining.min(bota_core::rules::MAX_ORDERS_PER_TICK);
        let messages = queue_moves(&mut game, count);
        assert!(
            !messages
                .iter()
                .any(|m| matches!(m, ServerMsg::OrderRejected { .. }))
        );
        remaining -= count;
    }
    let tick = game.tick();
    let messages = queue_moves(&mut game, 2);
    let refused: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            ServerMsg::OrderRejected { seq, reason } => Some((*seq, *reason)),
            _ => None,
        })
        .collect();
    assert_eq!(
        refused,
        vec![(tick * 128 + 1, bota_proto::RejectReason::QueueFull)]
    );
    let hero = game.hero_of(0).expect("hero");
    assert_eq!(
        game.world()
            .queue
            .get(hero)
            .expect("queued moves")
            .iter()
            .count(),
        bota_core::rules::MAX_QUEUED_ORDERS
    );
}

#[test]
fn a_match_at_its_time_cap_cancels_new_orders_without_repeating_its_last_tick() {
    let (mut game, mut opening) =
        LocalMatch::new_recorded(&configuration(), &[OrderDelay::NONE]).expect("recorded match");
    let mut first = opening.remove(0).into_iter();
    let Some(ServerMsg::MatchStart { info }) = first.next() else {
        panic!("match starts")
    };
    let mut client = bota_client_core::ClientSession::seat(
        bota_client_core::MatchStatic::new(&info).expect("match description"),
        SlotId(0),
        Some(HeroId::SHADOW_FIEND),
        bota_client_core::SessionOptions::default(),
    )
    .expect("strict player stream");
    for message in first {
        client.ingest(message, &mut ()).expect("opening stream");
    }
    game.take_records();
    let tick = game.tick();
    let gold = game.world().seats[0].gold;
    let hero = game.hero_of(0).expect("hero");
    game.world_mut().clock_shift.game = bota_core::MAP2_TICK_CAP - tick;
    let messages = step(
        &mut game,
        &[Order::Buy {
            item: ItemId::CIRCLET,
        }],
    );
    assert_eq!(game.tick(), tick, "a finished simulation advances no tick");
    assert_eq!(
        game.world().seats[0].gold,
        gold,
        "the cancelled purchase spends nothing"
    );
    assert!(
        game.world()
            .inventory
            .get(hero)
            .expect("hero inventory")
            .slots
            .iter()
            .flatten()
            .all(|item| item.id != ItemId::CIRCLET),
        "the cancelled purchase gives no item"
    );
    assert!(
        !messages
            .iter()
            .any(|m| matches!(m, ServerMsg::OrderRejected { .. })),
        "MatchOver cancels post-completion orders without a separate refusal"
    );
    assert!(messages.iter().any(|m| matches!(
        m,
        ServerMsg::MatchOver {
            winner: Team::Neutral,
            ..
        }
    )));
    for message in messages.iter().cloned() {
        client
            .ingest(message, &mut ())
            .expect("terminal stream remains valid for the real client");
    }
    assert!(
        !messages
            .iter()
            .any(|m| matches!(m, ServerMsg::Snapshot { .. } | ServerMsg::Events { .. })),
        "a completed world publishes no repeated tick"
    );
    let records = game.take_records();
    assert!(
        !records
            .iter()
            .any(|r| matches!(r, bota_proto::ReplayRecord::Orders { .. })),
        "no commands applied on a new tick"
    );
    assert!(
        !records
            .iter()
            .any(|r| matches!(r, bota_proto::ReplayRecord::Msg(ServerMsg::Snapshot { .. }))),
        "no repeated snapshot tick"
    );
    assert_eq!(records.len(), 1, "only the final result is recorded");
    assert!(matches!(
        &records[0],
        bota_proto::ReplayRecord::Msg(ServerMsg::MatchOver {
            winner: Team::Neutral,
            ..
        })
    ));
}

fn step(game: &mut LocalMatch, orders: &[Order]) -> Vec<ServerMsg> {
    let tick = game.tick();
    let given: Vec<_> = orders
        .iter()
        .enumerate()
        .map(|(index, order)| OutgoingOrder {
            seq: tick * 32 + index as u32,
            unit: None,
            order: *order,
            seen: tick,
            queue: false,
        })
        .collect();
    game.step(&[&given]).expect("one server tick").remove(0)
}

fn snapshot(messages: &[ServerMsg]) -> &WorldView {
    messages
        .iter()
        .find_map(|m| {
            if let ServerMsg::Snapshot { view } = m {
                Some(view)
            } else {
                None
            }
        })
        .expect("a snapshot")
}

#[test]
fn two_dependent_inventory_swaps_in_one_server_tick_both_apply_in_order() {
    let mut game = game();
    step(
        &mut game,
        &[Order::Cheat {
            cheat: Cheat::Item {
                item: ItemId::CIRCLET,
            },
        }],
    );
    let tick = game.tick();
    let messages = step(
        &mut game,
        &[
            Order::Swap {
                from: ItemSlot(0),
                to: ItemSlot(6),
            },
            Order::Swap {
                from: ItemSlot(6),
                to: ItemSlot(1),
            },
        ],
    );
    let refusals: Vec<_> = messages
        .iter()
        .filter_map(|m| {
            if let ServerMsg::OrderRejected { seq, reason } = m {
                Some((*seq, *reason))
            } else {
                None
            }
        })
        .collect();
    assert!(
        refusals.is_empty(),
        "both dependent transactions must be accepted: {refusals:?}"
    );
    let view = snapshot(&messages);
    assert_eq!(view.tick, tick + 1, "both swaps apply on one tick");
    let hero = view
        .unit(view.players[0].unit.expect("hero"))
        .expect("own hero");
    assert!(hero.items[0].is_none(), "first swap emptied the source");
    assert!(hero.items[6].is_none(), "second swap emptied the backpack");
    assert_eq!(
        hero.items[1].map(|i| i.id),
        Some(ItemId::CIRCLET),
        "second swap sees what the first deposited"
    );
}

#[test]
fn an_invalid_later_transaction_is_refused_against_the_updated_inventory() {
    let mut game = game();
    step(
        &mut game,
        &[Order::Cheat {
            cheat: Cheat::Item {
                item: ItemId::CIRCLET,
            },
        }],
    );
    let messages = step(
        &mut game,
        &[
            Order::Swap {
                from: ItemSlot(0),
                to: ItemSlot(6),
            },
            Order::Swap {
                from: ItemSlot(0),
                to: ItemSlot(1),
            },
        ],
    );
    let refusals: Vec<_> = messages
        .iter()
        .filter_map(|m| {
            if let ServerMsg::OrderRejected { reason, .. } = m {
                Some(*reason)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(refusals, vec![bota_proto::RejectReason::EmptySlot]);
    let view = snapshot(&messages);
    let hero = view
        .unit(view.players[0].unit.expect("hero"))
        .expect("hero");
    assert_eq!(hero.items[6].map(|i| i.id), Some(ItemId::CIRCLET));
    assert!(hero.items[0].is_none());
    assert!(hero.items[1].is_none());
}

#[test]
fn deferred_inventory_validation_still_rejects_foreign_units_and_invalid_slots() {
    let mut game = game();
    let tick = game.tick();
    let foreign = OutgoingOrder {
        seq: tick * 32,
        unit: Some(bota_proto::EntityId {
            idx: u32::MAX,
            generation: 0,
        }),
        order: Order::Swap {
            from: ItemSlot(0),
            to: ItemSlot(6),
        },
        seen: tick,
        queue: false,
    };
    let replies = game.step(&[&[foreign]]).expect("foreign request answered");
    assert!(replies[0].iter().any(|m| matches!(
        m,
        ServerMsg::OrderRejected {
            reason: bota_proto::RejectReason::NotYourUnit,
            ..
        }
    )));
    let messages = step(
        &mut game,
        &[Order::Swap {
            from: ItemSlot(255),
            to: ItemSlot(0),
        }],
    );
    assert!(messages.iter().any(|m| matches!(
        m,
        ServerMsg::OrderRejected {
            reason: bota_proto::RejectReason::EmptySlot,
            ..
        }
    )));
}

#[test]
fn same_tick_purchases_cannot_spend_the_same_gold_twice() {
    let mut game = game();
    let messages = step(
        &mut game,
        &[
            Order::Buy {
                item: ItemId::BOOTS,
            },
            Order::Buy {
                item: ItemId::BOOTS,
            },
        ],
    );
    let refusals: Vec<_> = messages
        .iter()
        .filter_map(|m| {
            if let ServerMsg::OrderRejected { reason, .. } = m {
                Some(*reason)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        refusals,
        vec![bota_proto::RejectReason::NotEnoughGold],
        "the second purchase is checked after the first spends gold"
    );
    let view = snapshot(&messages);
    let hero = view
        .unit(view.players[0].unit.expect("hero"))
        .expect("hero");
    assert_eq!(
        hero.items
            .iter()
            .flatten()
            .filter(|i| i.id == ItemId::BOOTS)
            .count(),
        1
    );
}

#[test]
fn buying_then_using_the_new_item_in_the_same_tick_checks_the_updated_bag() {
    let mut game = game();
    step(
        &mut game,
        &[Order::Cheat {
            cheat: Cheat::Gold { amount: 10000 },
        }],
    );
    let hero = game.hero_of(0).expect("hero");
    game.world_mut().mana.get_mut(hero).expect("mana").mana = bota_proto::Fixed::from_int(65);
    let messages = step(
        &mut game,
        &[
            Order::Buy {
                item: ItemId::MANGO,
            },
            Order::Use {
                slot: ItemSlot(0),
                target: bota_proto::Target::None,
            },
        ],
    );
    let refusals: Vec<_> = messages
        .iter()
        .filter_map(|m| {
            if let ServerMsg::OrderRejected { reason, .. } = m {
                Some(*reason)
            } else {
                None
            }
        })
        .collect();
    assert!(
        refusals.is_empty(),
        "the newly filled slot is usable: {refusals:?}"
    );
    let view = snapshot(&messages);
    let hero = view
        .unit(view.players[0].unit.expect("hero"))
        .expect("hero");
    assert!(
        hero.items[0].is_none(),
        "the admitted use really consumes the newly bought Mango"
    );
    assert!(hero.mana > 65, "the Mango restores mana");
}

#[test]
fn buying_intelligence_then_casting_checks_the_updated_mana_pool() {
    let mut game = game();
    step(
        &mut game,
        &[
            Order::Cheat {
                cheat: Cheat::Gold { amount: 10000 },
            },
            Order::Learn {
                slot: bota_proto::AbilitySlot(0),
            },
        ],
    );
    let hero = game.hero_of(0).expect("hero");
    game.world_mut().mana.get_mut(hero).expect("mana").mana = bota_proto::Fixed::from_int(65);
    let messages = step(
        &mut game,
        &[
            Order::Buy {
                item: ItemId::STAFF_OF_WIZARDRY,
            },
            Order::Cast {
                slot: bota_proto::AbilitySlot(0),
                target: bota_proto::Target::None,
            },
        ],
    );
    let refusals: Vec<_> = messages
        .iter()
        .filter_map(|m| {
            if let ServerMsg::OrderRejected { reason, .. } = m {
                Some(*reason)
            } else {
                None
            }
        })
        .collect();
    assert!(
        refusals.is_empty(),
        "the staff raises the current mana pool before the cast check: {refusals:?}"
    );
    let view = snapshot(&messages);
    assert!(
        view.unit(view.players[0].unit.expect("hero"))
            .expect("hero")
            .cast
            .is_some(),
        "the cast starts using the updated stats"
    );
}

#[test]
fn swapping_then_dropping_the_new_slot_in_the_same_tick_uses_the_updated_inventory() {
    let mut game = game();
    step(
        &mut game,
        &[Order::Cheat {
            cheat: Cheat::Item {
                item: ItemId::CIRCLET,
            },
        }],
    );
    let messages = step(
        &mut game,
        &[
            Order::Swap {
                from: ItemSlot(0),
                to: ItemSlot(1),
            },
            Order::Put {
                slot: ItemSlot(1),
                target: bota_proto::Target::None,
            },
        ],
    );
    let refusals: Vec<_> = messages
        .iter()
        .filter_map(|m| {
            if let ServerMsg::OrderRejected { reason, .. } = m {
                Some(*reason)
            } else {
                None
            }
        })
        .collect();
    assert!(
        refusals.is_empty(),
        "the drop sees the filled destination: {refusals:?}"
    );
    let view = snapshot(&messages);
    let hero = view
        .unit(view.players[0].unit.expect("hero"))
        .expect("hero");
    assert!(hero.items[0].is_none());
    assert!(hero.items[1].is_none());
    assert_eq!(
        view.loot.len(),
        1,
        "the ordered drop really reaches the ground"
    );
}
