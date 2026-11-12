mod support;

use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ModuleBank};
use bota_proto::{HeroId, Order, SlotId, Target, Team, Vec2};

fn team(usage: Option<&[u8]>) -> LuaTeam {
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    if let Some(source) = usage { modules.insert("ability_item_usage_nevermore", source).expect("usage"); }
    let mut lua = LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17,19], random: 23 }).expect("team");
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
        if tick == 0 { assert!(turn.bots[0].orders.iter().any(|order| matches!(order.order, Order::Learn { .. })), "default skill point must be spent"); }
        requested_cast |= turn.bots[0].orders.iter().any(|order| matches!(order.order, Order::Cast { .. }));
        let orders: Vec<_> = turn.bots[0].orders.iter().map(|&order| sessions[0].issue(order).expect("order")).collect();
        let opponent = sessions[1].issue(IssuedOrder::hero(Order::Attack { target: Target::Pos(Vec2::from_ints(9216,9216)) })).expect("opponent");
        let mut ended = false;
        for (session, stream) in sessions.iter_mut().zip(local.step(&[&orders, &[opponent]]).expect("step")) { ended |= support::fold(session, stream).is_some(); }
        cast_went |= sessions[0].own_hero().is_some_and(|own| own.abilities.iter().any(|ability| ability.cooldown_left > 0));
        if cast_went || ended { break; }
    }
    assert!(requested_cast);
    assert!(cast_went, "an emitted cast must actually reach the simulation");
}

#[test]
fn explicit_empty_auxiliary_hooks_remain_authoritative() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Some(b"function AbilityUsageThink() end function AbilityLevelUpThink() end"));
    let turn = lua.think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(!turn.bots[0].orders.iter().any(|order| matches!(order.order, Order::Learn { .. } | Order::Cast { .. })));
}
