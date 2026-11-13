mod support;
use bota_client_core::ClientSession;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam};
use bota_proto::{AbilityId, HeroId, ItemId, Order, SlotId, Team};

fn team(fail: bool) -> LuaTeam {
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
    lua.set_modules(support::courier_modules(fail))
        .expect("modules");
    lua.load_pack().expect("pack");
    lua
}
fn command(
    session: &ClientSession,
    order: &bota_client_core::IssuedOrder,
    ability: AbilityId,
) -> bool {
    let Some(courier) = session.own_courier() else {
        return false;
    };
    order.unit == Some(courier.id)
        && matches!(order.order, Order::Cast { slot, .. } if courier.abilities.get(usize::from(slot.0)).is_some_and(|entry| entry.id == ability))
}

#[test]
fn missing_courier_hooks_fetch_and_deliver_with_bounded_retries() {
    let (mut local, mut sessions) = support::courier_delivery_setup();
    let mut lua = team(false);
    let (mut fetched, mut delivered) = (0, 0);
    let mut received = false;
    let mut ticks = 0;
    for tick in 0..3000 {
        ticks = tick + 1;
        let turn = lua.think(&sessions[0]).expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        fetched += turn.bots[0]
            .orders
            .iter()
            .filter(|order| command(&sessions[0], order, AbilityId::TAKE_STASH))
            .count();
        delivered += turn.bots[0]
            .orders
            .iter()
            .filter(|order| command(&sessions[0], order, AbilityId::DELIVER))
            .count();
        if tick == 0 {
            assert_eq!(fetched, 1, "stash needs a courier request");
        }
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
        received |= sessions[0]
            .own_hero()
            .expect("hero")
            .items
            .iter()
            .flatten()
            .any(|item| item.id == ItemId::IRON_BRANCH);
        if received {
            break;
        }
    }
    assert!(received);
    assert_eq!(fetched, 1);
    assert!(
        delivered > 0 && delivered < ticks / 2 + 2,
        "courier commands must not repeat each tick"
    );
}

#[test]
fn a_later_hook_error_does_not_consume_a_native_courier_request() {
    let (mut local, mut sessions) = support::courier_delivery_setup();
    let mut lua = team(true);
    let rejected = lua.think(&sessions[0]).expect("first turn");
    assert_eq!(rejected.failures.len(), 1);
    assert!(rejected.bots[0].orders.is_empty());
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[], &[]]).expect("next tick"))
    {
        support::fold(session, stream);
    }
    let retried = lua.think(&sessions[0]).expect("retry");
    assert!(retried.failures.is_empty(), "{:?}", retried.failures);
    assert!(retried.bots[0].orders.iter().any(|order| command(
        &sessions[0],
        order,
        AbilityId::TAKE_STASH
    )));
}

#[test]
fn an_explicit_minion_hook_exclusively_controls_the_courier() {
    let (_, sessions) = support::courier_delivery_setup();
    for source in [
        "function MinionThink(unit) end",
        "function MinionThink(unit) unit:Action_MoveToLocation(Vector(0,0,0)) end",
    ] {
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
        let modules = support::courier_modules(false);

        lua.set_modules(modules).expect("modules");
        lua.load_pack().expect("pack");
        lua.add_script(
            SlotId(0),
            bota_lua::ScriptKind::Minion,
            "=minion_override",
            source.as_bytes(),
        )
        .expect("minion source");
        let turn = lua.think(&sessions[0]).expect("turn");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert!(
            !turn.bots[0].orders.iter().any(|order| command(
                &sessions[0],
                order,
                AbilityId::TAKE_STASH
            )),
            "explicit minion behavior replaces native courier management"
        );
        let courier = sessions[0].own_courier().expect("courier").id;
        let moves = turn.bots[0]
            .orders
            .iter()
            .filter(|order| {
                order.unit == Some(courier) && matches!(order.order, Order::Move { .. })
            })
            .count();
        assert_eq!(moves, usize::from(source.contains("Action_MoveToLocation")));
    }
}
