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
            let committed = turn
                .bots
                .iter()
                .filter(|bot| !bot.orders.is_empty())
                .count();
            eprintln!(
                "pack_first_failure team={team:?} tick={ticks} reported_errors={} additional_team_errors={} bots_with_committed_orders={committed} deadline_ticks={} retired={}",
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

#[test]
#[ignore = "requires BOTA_LUA_LOCAL_PACK; optional local source conformance"]
fn local_pack_issues_a_body_order_through_the_host_before_the_first_minute() {
    first_order_proof(false);
}

#[test]
#[ignore = "requires BOTA_LUA_LOCAL_PACK; optional local source conformance"]
fn local_pack_reads_its_controller_and_issues_orders_without_a_controller_api_failure() {
    first_order_proof(true);
}

fn first_order_proof(require_controller: bool) {
    let directory =
        std::env::var_os("BOTA_LUA_LOCAL_PACK").expect("BOTA_LUA_LOCAL_PACK is required");
    let runtime = std::env::var_os("BOTA_LUA_RUNTIME_SCRIPTS").map(std::path::PathBuf::from);
    for side in [Team::Radiant, Team::Dire] {
        let index = usize::from(side == Team::Dire);
        let limits = NativeConfig::new(
            side,
            vec![BotSeat {
                slot: SlotId(index as u8),
                hero: HeroId::SHADOW_FIEND,
            }],
            bota_lua::Keys {
                hash: [17, 19],
                random: 23,
            },
        )
        .limits;
        let modules = load_modules(std::path::Path::new(&directory), runtime.as_deref(), limits)
            .expect("bounded local sources");
        let mut lua = bota_lua::LuaTeam::new_measured(
            side,
            &[BotSeat {
                slot: SlotId(index as u8),
                hero: HeroId::SHADOW_FIEND,
            }],
            limits,
            bota_lua::Keys {
                hash: [17, 19],
                random: 23,
            },
            true,
        )
        .unwrap();
        lua.set_modules(modules).unwrap();
        lua.load_pack().unwrap();
        let (mut local, mut sessions) = if require_controller {
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID)
        } else {
            support::local(&[Team::Radiant, Team::Dire])
        };
        if require_controller {
            let at = Vec2::from_ints(9216, 9216);
            let setup = [
                bota_proto::Cheat::Teleport { at },
                bota_proto::Cheat::StartGame,
                bota_proto::Cheat::CreateCreeps {
                    enemy: true,
                    at: at + Vec2::from_ints(224, 0),
                },
            ];
            let outgoing: Vec<_> = setup
                .into_iter()
                .map(|cheat| {
                    sessions[index]
                        .issue(IssuedOrder::hero(Order::Cheat { cheat }))
                        .unwrap()
                })
                .collect();
            let mut commands = [Vec::new(), Vec::new()];
            commands[index] = outgoing;
            for (session, stream) in sessions
                .iter_mut()
                .zip(local.step(&[&commands[0], &commands[1]]).unwrap())
            {
                support::fold(session, stream);
            }
        }
        let mut first_error = None;
        let mut issued = false;
        let mut saw_controller = !require_controller;
        let mut gaps = Vec::new();
        // Pack objectives contain clock gates. Observe the real Solo Mid pregame
        // plus one bounded game minute; no pack source is changed.
        for _ in 0..bota_core::map_of(bota_core::MAP2_ID).pregame_ticks
            + 60 * bota_core::rules::TICKS_PER_SECOND
        {
            let turn = lua.think(&sessions[index]).expect("script host turn");
            if require_controller {
                gaps = turn
                    .statistics
                    .as_ref()
                    .unwrap()
                    .undefined_api_reads
                    .names
                    .clone();
                for failure in &turn.failures {
                    let message = failure.error.to_string();
                    assert!(
                        ![
                            "GetCurrentActionType",
                            "ActionQueue_AttackUnit",
                            "GetQueuedActionType",
                            "NumQueuedActions"
                        ]
                        .iter()
                        .any(|name| message.contains(name)),
                        "controller API failure: {message}"
                    );
                }
                saw_controller |= turn
                    .statistics
                    .as_ref()
                    .unwrap()
                    .api
                    .iter()
                    .any(|api| api.name == "GetCurrentActionType" && api.calls > 0);
            }
            if first_error.is_none() {
                first_error = turn
                    .failures
                    .first()
                    .map(|failure| failure.error.to_string());
            }
            let outgoing: Vec<_> = turn.bots[0]
                .orders
                .iter()
                .map(|&order| sessions[index].issue(order).unwrap())
                .collect();
            let scripted = turn.statistics.as_ref().unwrap().api.iter().any(|api| {
                api.calls > 0
                    && (api.name.starts_with("Action_Move")
                        || api.name.starts_with("Action_Attack")
                        || api.name.starts_with("Action_UseAbility"))
            });
            issued |= scripted
                && turn.bots[0].orders.iter().any(|order| {
                    order.unit.is_none()
                        && matches!(
                            order.order,
                            Order::Move { .. }
                                | Order::Attack { .. }
                                | Order::Cast { .. }
                                | Order::Use { .. }
                        )
                });
            let mut commands = [Vec::new(), Vec::new()];
            commands[index] = outgoing;
            for (session, stream) in sessions
                .iter_mut()
                .zip(local.step(&[&commands[0], &commands[1]]).unwrap())
            {
                support::fold(session, stream);
            }
            if issued && saw_controller {
                println!(
                    "pack_first_order side={side:?} tick={} failures_so_far={:?}",
                    sessions[index].current().unwrap().tick,
                    first_error
                );
                break;
            }
        }
        assert!(
            issued && saw_controller,
            "side={side:?}: no script body order through the host; first_error={first_error:?}; gaps={gaps:?}"
        );
    }
}
