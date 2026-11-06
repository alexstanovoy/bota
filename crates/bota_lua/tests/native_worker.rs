#![cfg(all(feature = "native", not(target_arch = "wasm32")))]

mod support;

use std::time::Duration;

use bota_lua::{BotSeat, Keys, ModuleBank, Limits};
use bota_lua::native::{NativeConfig, NativeError, NativeTeam};
use bota_proto::{HeroId, Order, SlotId, Target, Team};
use bota_client_core::IssuedOrder;

fn config() -> NativeConfig {
    let mut config = NativeConfig::new(Team::Radiant, vec![BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Keys { hash: [17, 19], random: 23 });
    config.think_timeout = Duration::from_secs(10);
    config
}

fn modules() -> ModuleBank {
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    modules.insert("bots/bot_nevermore", include_bytes!("scripts/duel.lua")).expect("script");
    modules
}

#[test]
fn native_worker_plays_shadow_fiend_to_match_over_through_real_orders() {
    let mut lua = NativeTeam::new(config(), modules()).expect("worker");
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut sent = 0;
    let mut winner = None;
    for _ in 0..bota_core::MAP2_TICK_CAP {
        let turn = lua.think(&sessions[0]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        let orders: Vec<_> = turn.bots[0].orders.iter().map(|&order| sessions[0].issue(order).expect("ordinary order")).collect();
        sent += orders.len();
        let opponent = sessions[1].issue(IssuedOrder::hero(Order::Attack { target: Target::Pos(bota_proto::Vec2::from_ints(9216,9216)) })).expect("opponent");
        let streams = local.step(&[&orders, &[opponent]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) { winner = support::fold(session, stream).or(winner); }
        if winner.is_some() { break; }
    }
    assert!(sent > 0);
    assert!(winner.is_some());
}

#[test]
fn an_immediate_deadline_retires_the_worker_without_committing_orders() {
    let mut config = config();
    config.think_timeout = Duration::ZERO;
    let mut lua = NativeTeam::new(config, modules()).expect("worker");
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    assert!(matches!(lua.think(&sessions[0]), Err(NativeError::Deadline)));
    assert!(lua.retired());
    assert!(matches!(lua.think(&sessions[0]), Err(NativeError::Retired)));
}
