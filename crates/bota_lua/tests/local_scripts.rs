#![cfg(all(feature = "native", target_os = "linux"))]
mod support;
use bota_client_core::IssuedOrder;
use bota_lua::{
    BotSeat, Limits,
    native::{NativeConfig, NativeTeam, load_modules, random_keys},
};
use bota_proto::{HeroId, Order, SlotId, Target, Team, Vec2};

#[test]
#[ignore = "requires BOTA_LUA_LOCAL_PACK; optional local source conformance"]
fn local_scripts_play_a_complete_match_without_script_failures() {
    let directory =
        std::env::var_os("BOTA_LUA_LOCAL_PACK").expect("BOTA_LUA_LOCAL_PACK is required");
    let runtime = std::env::var_os("BOTA_LUA_RUNTIME_SCRIPTS").map(std::path::PathBuf::from);
    let modules = load_modules(
        std::path::Path::new(&directory),
        runtime.as_deref(),
        Limits::DEFAULT,
    )
    .expect("local sources");
    let mut keys = random_keys().expect("hash entropy");
    keys.random = 3;
    let mut config = NativeConfig::new(
        Team::Radiant,
        vec![BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        keys,
    );
    config.think_timeout = std::time::Duration::from_secs(10);
    let mut lua = NativeTeam::new(config, modules).expect("worker");
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut sent = 0;
    let mut ended = false;
    for _ in 0..bota_core::MAP2_TICK_CAP {
        let turn = lua.think(&sessions[0]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        let orders: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .map(|&order| sessions[0].issue(order).expect("order"))
            .collect();
        sent += orders.len();
        let opponent = sessions[1]
            .issue(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(Vec2::from_ints(9216, 9216)),
            }))
            .expect("opponent");
        let streams = local.step(&[&orders, &[opponent]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            ended |= support::fold(session, stream).is_some();
        }
        if ended {
            break;
        }
    }
    assert!(sent > 0);
    assert!(ended);
}
