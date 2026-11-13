#![cfg(all(feature = "native", not(target_arch = "wasm32")))]

mod support;

use std::time::Duration;

use bota_client_core::IssuedOrder;
use bota_lua::native::{NativeConfig, NativeError, NativeTeam};
use bota_lua::{BotSeat, Keys, Limits, ModuleBank};
use bota_proto::{HeroId, Order, SlotId, Target, Team};

fn config() -> NativeConfig {
    let mut config = NativeConfig::new(
        Team::Radiant,
        vec![BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Keys {
            hash: [17, 19],
            random: 23,
        },
    );
    config.think_timeout = Duration::from_secs(10);
    config
}

fn modules() -> ModuleBank {
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    modules
        .insert("bots/bot_nevermore", include_bytes!("scripts/duel.lua"))
        .expect("script");
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
        let orders: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .map(|&order| sessions[0].issue(order).expect("ordinary order"))
            .collect();
        sent += orders.len();
        let opponent = sessions[1]
            .issue(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(bota_proto::Vec2::from_ints(9216, 9216)),
            }))
            .expect("opponent");
        let streams = local.step(&[&orders, &[opponent]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            winner = support::fold(session, stream).or(winner);
        }
        if winner.is_some() {
            break;
        }
    }
    assert!(sent > 0);
    assert!(winner.is_some());
}

#[test]
fn consecutive_deadlines_retire_the_worker_without_committing_orders() {
    let mut config = config();
    config.think_timeout = Duration::ZERO;
    let mut lua = NativeTeam::new(config, modules()).expect("worker");
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for tick in 0..3 {
        assert!(matches!(
            lua.think(&sessions[0]),
            Err(NativeError::Deadline)
        ));
        assert_eq!(
            lua.retired(),
            tick == 2,
            "retire only after three consecutive missed ticks"
        );
        let streams = local.step(&[&[], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
    assert!(lua.retired());
    assert!(matches!(lua.think(&sessions[0]), Err(NativeError::Retired)));
}

#[test]
fn a_pack_without_a_takeover_loads_its_generic_mode_override() {
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    modules.insert("bots/mode_farm_generic",b"function GetDesire() return 0.5 end function Think() assert(GetBot():GetActiveMode()==BOT_MODE_FARM) GetBot():Action_MoveToLocation(Vector(0,0,0)) end").expect("mode");
    let mut lua = NativeTeam::new(config(), modules).expect("worker");
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders,vec![
        bota_client_core::IssuedOrder::hero(bota_proto::Order::Move {target:bota_proto::Target::Pos(bota_proto::Vec2::from_ints(9216,9216))}),
        bota_client_core::IssuedOrder::hero(bota_proto::Order::Learn {slot:bota_proto::AbilitySlot(0)}),
        bota_client_core::IssuedOrder::hero(bota_proto::Order::Buy {item:bota_proto::ItemId::IRON_BRANCH}),
    ],"the explicit farm movement keeps the missing learning and purchase defaults");
}

#[test]
fn default_deadline_keeps_four_fuel_exhausting_bots_available() {
    let seats: Vec<_> = (0..4)
        .map(|slot| BotSeat {
            slot: SlotId(slot * 2),
            hero: HeroId::SHADOW_FIEND,
        })
        .collect();
    let config = NativeConfig::new(
        Team::Radiant,
        seats,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    );
    assert!(
        u128::from(config.limits.fuel_per_tick) * 40 < config.think_timeout.as_nanos(),
        "fuel must leave margin beyond measured13ns per quantum"
    );
    let mut sources = ModuleBank::new(Limits::DEFAULT);
    sources
        .insert(
            "bots/bot_nevermore",
            b"function Think() local t={} local i=0 while true do i=i+1 t[i]={} end end",
        )
        .expect("source");
    let mut native = NativeTeam::new(config, sources).expect("worker");
    let (mut local, mut sessions) = support::local(&[
        Team::Radiant,
        Team::Dire,
        Team::Radiant,
        Team::Dire,
        Team::Radiant,
        Team::Dire,
        Team::Radiant,
    ]);
    let mut completed = 0;
    for _ in 0..24 {
        match native.think(&sessions[0]) {
            Ok(turn) => {
                assert_eq!(turn.failures.len(), 4);
                assert!(turn.bots.iter().all(|bot| bot.orders.is_empty()));
                completed += 1;
            }
            Err(NativeError::Deadline) => {}
            Err(error) => panic!("unexpected worker failure: {error}"),
        }
        assert!(
            !native.retired(),
            "bounded per-bot fuel exhaustion must not lose the team"
        );
        let streams = local
            .step(&[&[], &[], &[], &[], &[], &[], &[]])
            .expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
    assert!(
        completed >= 12,
        "normal fuel-bounded ticks complete at realistic33ms deadlines"
    );
}
