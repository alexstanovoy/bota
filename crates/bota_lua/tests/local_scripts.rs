#![cfg(all(feature = "native", target_os = "linux"))]
mod support;
use bota_client_core::IssuedOrder;
use bota_lua::{
    BotSeat, Limits,
    native::{HeroSelection, NativeConfig, NativeTeam, load_modules, random_keys},
};
use bota_proto::{HeroId, Order, SlotId, Target, Team, Vec2};

#[test]
#[ignore = "requires BOTA_LUA_LOCAL_PACK; optional local source conformance"]
fn local_scripts_play_a_complete_match_without_script_failures() {
    let directory =
        std::env::var_os("BOTA_LUA_LOCAL_PACK").expect("BOTA_LUA_LOCAL_PACK is required");
    let runtime = std::env::var_os("BOTA_LUA_RUNTIME_SCRIPTS").map(std::path::PathBuf::from);
    let sides: &[Team] = match std::env::var("BOTA_LUA_LOCAL_TEAM").as_deref() {
        Ok("radiant") => &[Team::Radiant],
        Ok("dire") => &[Team::Dire],
        Err(std::env::VarError::NotPresent) => &[Team::Radiant, Team::Dire],
        _ => panic!("BOTA_LUA_LOCAL_TEAM must be radiant or dire when set"),
    };
    for &team in sides {
        run_pack(std::path::Path::new(&directory), runtime.as_deref(), team);
    }
}

fn run_pack(directory: &std::path::Path, runtime: Option<&std::path::Path>, team: Team) {
    let index = usize::from(team == Team::Dire);
    let modules = load_modules(directory, runtime, Limits::DEFAULT).expect("local sources");
    let mut keys = random_keys().expect("hash entropy");
    keys.random = 3;
    let mut config = NativeConfig::new(
        team,
        vec![BotSeat {
            slot: SlotId(index as u8),
            hero: HeroId::SHADOW_FIEND,
        }],
        keys,
    );
    config.hero_selection = HeroSelection::Fixed;
    config.think_timeout = std::time::Duration::from_secs(10);
    let mut lua = NativeTeam::new(config, modules).expect("worker");
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut activity = bota_lua::HookActivity::default();
    let mut sent = 0;
    let mut ended = false;
    let mut ticks = 0;
    for _ in 0..bota_core::MAP2_TICK_CAP {
        ticks += 1;
        let turn = lua.think(&sessions[index]).expect("think");
        if !turn.failures.is_empty() {
            let discarded = turn
                .bots
                .iter()
                .filter(|bot| turn.failures.iter().any(|failure| failure.slot == bot.slot))
                .count();
            assert!(
                turn.bots
                    .iter()
                    .filter(|bot| { turn.failures.iter().any(|failure| failure.slot == bot.slot) })
                    .all(|bot| bot.orders.is_empty()),
                "a failed turn cannot leak orders"
            );
            eprintln!(
                "pack_first_failure team={team:?} tick={ticks} reported_errors={} additional_team_errors={} discarded_bot_turns={discarded} deadline_ticks={} retired={}",
                turn.failures.len(),
                turn.hook_activity.suppressed_team_errors,
                lua.deadline_ticks(),
                lua.retired()
            );
        }
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        activity += turn.hook_activity;
        let orders: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .map(|&order| sessions[index].issue(order).expect("order"))
            .collect();
        sent += orders.len();
        let opponent = sessions[1 - index]
            .issue(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(Vec2::from_ints(9216, 9216)),
            }))
            .expect("opponent");
        let mut by_seat = [Vec::new(), Vec::new()];
        by_seat[index] = orders;
        by_seat[1 - index].push(opponent);
        let streams = local.step(&[&by_seat[0], &by_seat[1]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            ended |= support::fold(session, stream).is_some();
        }
        if ended {
            break;
        }
    }
    assert!(sent > 0);
    assert!(ended);
    assert!(!lua.retired());
    assert_eq!(
        lua.deadline_ticks(),
        0,
        "conformance cannot discard late turns"
    );
    println!(
        "pack_conformance team={team:?} ticks={ticks} orders={sent} script_failures=0 retired=false script_initializations={} script_hooks={} native_hooks={} suppressed_team_errors={} deadline_ticks={}",
        activity.initializations,
        activity.scripted,
        activity.native,
        activity.suppressed_team_errors,
        lua.deadline_ticks()
    );
}
