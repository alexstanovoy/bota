mod support;

use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ModuleBank};
use bota_proto::{HeroId, Order, SlotId, Target, Team, Vec2};

fn team(usage: Option<&[u8]>) -> LuaTeam {
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    if let Some(source) = usage {
        modules
            .insert("ability_item_usage_nevermore", source)
            .expect("usage");
    }
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.set_modules(modules).expect("modules");
    lua.load_pack().expect("pack");
    lua
}

#[test]
fn missing_auxiliary_hooks_learn_and_cast_through_real_orders() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(None);
    let mut requested_cast = false;
    let mut cast_went = false;
    for tick in 0..8000 {
        let turn = lua.think(&sessions[0]).expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        if tick == 0 {
            assert!(
                turn.bots[0]
                    .orders
                    .iter()
                    .any(|order| matches!(order.order, Order::Learn { .. })),
                "default skill point must be spent"
            );
        }
        requested_cast |= turn.bots[0]
            .orders
            .iter()
            .any(|order| matches!(order.order, Order::Cast { .. }));
        let orders: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .map(|&order| sessions[0].issue(order).expect("order"))
            .collect();
        let opponent = sessions[1]
            .issue(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(Vec2::from_ints(9216, 9216)),
            }))
            .expect("opponent");
        let mut ended = false;
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&orders, &[opponent]]).expect("step"))
        {
            ended |= support::fold(session, stream).is_some();
        }
        cast_went |= sessions[0].own_hero().is_some_and(|own| {
            own.abilities
                .iter()
                .any(|ability| ability.cooldown_left > 0)
        });
        if cast_went || ended {
            break;
        }
    }
    assert!(requested_cast);
    assert!(
        cast_went,
        "an emitted cast must actually reach the simulation"
    );
}

#[test]
fn explicit_empty_auxiliary_hooks_remain_authoritative() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Some(
        b"function AbilityUsageThink() end function AbilityLevelUpThink() end",
    ));
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(!turn.bots[0].orders.iter().any(|order| order.unit.is_none()
        && matches!(order.order, Order::Learn { .. } | Order::Cast { .. })));
}

#[test]
fn default_items_use_ready_carried_regeneration_without_repeating_its_active_effect() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for _ in 0..2 {
        let buy = sessions[0]
            .issue(IssuedOrder::hero(Order::Buy {
                item: bota_proto::ItemId::HEALING_SALVE,
            }))
            .expect("buy");
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&[buy], &[]]).expect("purchase"))
        {
            support::fold(session, stream);
        }
    }
    let walk = sessions[0]
        .issue(IssuedOrder::hero(Order::Move {
            target: Target::Pos(Vec2::from_ints(9216, 9216)),
        }))
        .expect("walk");
    for tick in 0..250 {
        let orders = if tick == 0 { &[walk][..] } else { &[] };
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[orders, &[]]).expect("walk step"))
        {
            support::fold(session, stream);
        }
    }
    let mut lua = team(None);
    low_health_step(&mut local, &mut sessions);
    let first = lua.think(&sessions[0]).expect("item turn");
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    let use_item = first.bots[0]
        .orders
        .iter()
        .find(|order| matches!(order.order, Order::Use { .. }))
        .expect("regeneration use");
    let issued = sessions[0].issue(*use_item).expect("use");
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[issued], &[]]).expect("use step"))
    {
        support::fold(session, stream);
    }
    assert!(
        sessions[0]
            .own_hero()
            .expect("hero")
            .effects
            .iter()
            .any(|effect| effect.inflictor
                == Some(bota_proto::Inflictor::Item(
                    bota_proto::ItemId::HEALING_SALVE
                )))
    );
    low_health_step(&mut local, &mut sessions);
    let second = lua.think(&sessions[0]).expect("active effect turn");
    assert!(second.failures.is_empty(), "{:?}", second.failures);
    assert!(
        !second.bots[0]
            .orders
            .iter()
            .any(|order| matches!(order.order, Order::Use { .. }))
    );
}

#[test]
fn missing_purchase_hooks_equip_the_bot_without_rebuying_completed_starting_goals() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let initial = sessions[0].current().expect("view").players[0]
        .gold
        .expect("own gold");
    let mut lua = team(None);
    let mut branch_buys = 0;
    for tick in 0..20 {
        let turn = lua.think(&sessions[0]).expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        let purchases: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .filter(|order| matches!(order.order, Order::Buy { .. }))
            .collect();
        if tick == 0 {
            assert_eq!(purchases.len(), 1, "an unoverridden bot must equip itself");
        }
        branch_buys += purchases.iter().filter(|order| matches!(order.order, Order::Buy { item } if bota_core::item_by_key("item_branches") == Some(item))).count();
        let orders: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .map(|&order| sessions[0].issue(order).expect("order"))
            .collect();
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&orders, &[]]).expect("step"))
        {
            support::fold(session, stream);
        }
    }
    assert_eq!(branch_buys, 2);
    assert!(
        sessions[0].current().expect("view").players[0]
            .gold
            .expect("own gold")
            < initial
    );
    assert!(
        sessions[0]
            .own_hero()
            .expect("hero")
            .items
            .iter()
            .flatten()
            .count()
            >= 2
    );
}

#[test]
fn upgraded_equipment_satisfies_its_component_goals() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let branch = bota_core::item_by_key("item_branches").expect("branch");
    let buy = sessions[0]
        .issue(IssuedOrder::hero(Order::Buy { item: branch }))
        .expect("buy");
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[buy], &[]]).expect("step"))
    {
        support::fold(session, stream);
    }
    let own = sessions[0].own_hero().expect("hero");
    let source = own.id;
    let template = *own.items.iter().flatten().next().expect("item template");
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    let tick = view.tick;
    view.players[0].gold = Some(10000);
    let own = view
        .units
        .iter_mut()
        .find(|unit| unit.id == source)
        .expect("hero");
    own.items.fill(None);
    for (slot, key) in [
        "item_branches",
        "item_branches",
        "item_wraith_band",
        "item_bottle",
        "item_power_treads",
    ]
    .into_iter()
    .enumerate()
    {
        let id = bota_core::item_by_key(key).expect("equipment");
        let definition = bota_core::item_def(id).expect("definition");
        own.items[slot] = Some(bota_proto::ItemView {
            id,
            mode: definition.mode,
            charges: (definition.charges > 0).then_some(definition.charges),
            ..template
        });
    }
    sessions[0]
        .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
        .expect("equipment observation");
    sessions[0]
        .ingest(
            bota_proto::ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    let turn = team(None).think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    let wand = bota_core::item_by_key("item_magic_wand").expect("wand");
    let boots = bota_core::item_by_key("item_boots").expect("boots");
    assert!(
        turn.bots[0]
            .orders
            .iter()
            .any(|order| order.order == Order::Buy { item: wand })
    );
    assert!(
        !turn.bots[0]
            .orders
            .iter()
            .any(|order| order.order == Order::Buy { item: boots })
    );
}

fn low_health_step(
    local: &mut bota_server::local::LocalMatch,
    sessions: &mut [bota_client_core::ClientSession],
) {
    let source = sessions[0].own_hero().expect("hero").id;
    let mut streams = local.step(&[&[], &[]]).expect("observation step");
    for message in &mut streams[0] {
        if let bota_proto::ServerMsg::Snapshot { view } = message {
            view.units
                .iter_mut()
                .find(|unit| unit.id == source)
                .expect("hero")
                .hp = 1;
        }
    }
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
}
