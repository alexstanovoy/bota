use super::*;
use crate::state::{Lane, LaneDesires};
fn values(desires: Option<LaneDesires>) -> Option<[f64; 3]> {
    desires.map(|desires| {
        [
            desires.get(Lane::Top),
            desires.get(Lane::Mid),
            desires.get(Lane::Bot),
        ]
    })
}
mod support {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
}

fn team(source: &str) -> LuaTeam {
    team_with_limits(source, Limits::DEFAULT)
}
fn team_with_limits(source: &str, limits: Limits) -> LuaTeam {
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        limits,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.add_script(
        SlotId(0),
        ScriptKind::TeamDesires,
        "=team_desires",
        source.as_bytes(),
    )
    .expect("script");
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "=idle",
        b"function Think() end",
    )
    .expect("idle takeover");
    lua.set_gc_stress(luagee::GcStress::Full);
    lua
}
fn advance(local: &mut bota_server::local::LocalMatch, sessions: &mut [ClientSession]) {
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[], &[]]).expect("next tick"))
    {
        support::fold(session, stream);
    }
}

#[test]
fn team_desire_pairs_reset_missing_returns_to_native_policy() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(
        "local frame=0 function TeamThink() frame=frame+1 GetBot():Action_MoveToLocation(Vector(0,0,0)) end function UpdatePushLaneDesires() if frame==1 then return {0.125,0.375,0.625} end end function UpdateDefendLaneDesires() return {0.75,0.5,0.25} end",
    );
    let first = lua.think(&sessions[0]).expect("first turn");
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    assert!(first.bots[0].orders.iter().any(|order| order.order
        == bota_proto::Order::Move {
            target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(9216, 9216))
        }));
    assert_eq!(
        values(lua.state.borrow().team_desires.push),
        Some([0.125, 0.375, 0.625])
    );
    assert_eq!(
        values(lua.state.borrow().team_desires.defend),
        Some([0.75, 0.5, 0.25])
    );
    advance(&mut local, &mut sessions);
    let next = lua.think(&sessions[0]).expect("next turn");
    assert!(next.failures.is_empty(), "{:?}", next.failures);
    assert_eq!(values(lua.state.borrow().team_desires.push), None);
    assert_eq!(
        values(lua.state.borrow().team_desires.defend),
        Some([0.75, 0.5, 0.25])
    );
}

#[test]
fn malformed_team_desires_reject_the_turn_without_publishing_partial_updates() {
    for bad in [
        "false",
        "{}",
        "'not a table'",
        "5",
        "{[1]=0.1,[3]=0.3}",
        "{0.1,'not a number',0.3}",
        "{0.1,0.2,0.3,0.4}",
        "{0.1,0.2,0.3,nil,0.5}",
        "{[1]=0.1,[2]=0.2,[3]=0.3,[1000000]=0.5}",
        "{0.1,0.2,0.3,extra=true}",
        "{0.1,-math.huge,0.3}",
        "{0.1,0.2}",
        "{0.1,0/0,0.3}",
        "{0.1,math.huge,0.3}",
        "{0.1,-0.1,0.3}",
        "{0.1,1.1,0.3}",
        "setmetatable({}, {__index=function() error('must not run') end})",
    ] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let source = format!(
            "local frame=0 function TeamThink() frame=frame+1 GetBot():Action_MoveToLocation(Vector(0,0,0)) end function UpdatePushLaneDesires() if frame==1 then return {{0.125,0.375,0.625}} end return {{0.5,0.25,0.125}} end function UpdateDefendLaneDesires() if frame==2 then return {bad} end return {{0.75,0.5,0.25}} end"
        );
        let mut lua = team(&source);
        assert!(
            lua.think(&sessions[0])
                .expect("valid turn")
                .failures
                .is_empty()
        );
        let before = lua.state.borrow().team_desires;
        advance(&mut local, &mut sessions);
        let rejected = lua.think(&sessions[0]).expect("invalid return");
        assert_eq!(rejected.failures.len(), 1, "{bad}");
        assert!(
            matches!(&rejected.failures[0].error, Error::Runtime { message, .. } if message.contains("team lane desires")),
            "{:?}",
            rejected.failures
        );
        assert!(rejected.bots[0].orders.is_empty());
        assert_eq!(lua.state.borrow().team_desires, before, "{bad}");
        advance(&mut local, &mut sessions);
        assert!(
            lua.think(&sessions[0])
                .expect("recovered turn")
                .failures
                .is_empty()
        );
    }
}

#[test]
fn unrelated_team_hook_errors_do_not_freeze_valid_lane_desires() {
    for hook in [
        "TeamThink",
        "UpdateFarmLaneDesires",
        "UpdateRoamDesire",
        "UpdateRoshanDesire",
    ] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let source = format!(
            "local frame=0 function UpdatePushLaneDesires() frame=frame+1 return {{frame*0.125,0,1}} end function UpdateDefendLaneDesires() return {{1,0,0.5}} end function {hook}() error('unrelated hook') end"
        );
        let mut lua = team(&source);
        for expected in [0.125, 0.25, 0.375] {
            let turn = lua.think(&sessions[0]).expect("team hook error");
            assert_eq!(turn.failures.len(), 1, "{hook}");
            assert!(
                matches!(&turn.failures[0].error, Error::Runtime { message, .. } if message.contains("unrelated hook"))
            );
            assert_eq!(
                values(lua.state.borrow().team_desires.push),
                Some([expected, 0.0, 1.0]),
                "{hook}"
            );
            assert_eq!(
                values(lua.state.borrow().team_desires.defend),
                Some([1.0, 0.0, 0.5]),
                "{hook}"
            );
            advance(&mut local, &mut sessions);
        }
    }
}

#[test]
fn decoded_team_desires_normalize_zero_sign_and_accept_closed_boundaries() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(
        "function UpdatePushLaneDesires() return {-0.0,0,1} end function UpdateDefendLaneDesires() return {1,0,-0.0} end",
    );
    let turn = lua.think(&sessions[0]).expect("boundary return");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    let desires = lua.state.borrow().team_desires;
    assert_eq!(values(desires.push), Some([0.0, 0.0, 1.0]));
    assert_eq!(values(desires.defend), Some([1.0, 0.0, 0.0]));
    assert_eq!(
        values(desires.push).expect("push")[0].to_bits(),
        0.0f64.to_bits()
    );
    assert_eq!(
        values(desires.defend).expect("defend")[2].to_bits(),
        0.0f64.to_bits()
    );
}

#[test]
fn failed_push_keeps_an_empty_initial_pair_but_does_not_skip_valid_defend() {
    for failure in [
        "return false",
        "error('push hook failure')",
        "while true do end",
    ] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let source = format!(
            "local pushes=0 local defends=0 function UpdatePushLaneDesires() pushes=pushes+1 if pushes==1 then {failure} end return {{0,0.5,1}} end function UpdateDefendLaneDesires() defends=defends+1 return {{defends*0.125,0,1}} end"
        );
        let mut lua = team_with_limits(
            &source,
            Limits {
                fuel_per_call: 10_000,
                fuel_per_tick: 500_000,
                ..Limits::DEFAULT
            },
        );
        let failed = lua.think(&sessions[0]).expect("first-frame failure");
        assert_eq!(failed.failures.len(), 1, "{failure}");
        assert!(failed.bots[0].orders.is_empty());
        assert_eq!(
            lua.state.borrow().team_desires,
            crate::state::TeamDesires::default()
        );
        if failure.starts_with("return") {
            assert!(
                matches!(&failed.failures[0].error, Error::Runtime { message,.. } if message.contains("team lane desires")),
                "{:?}",
                failed.failures
            );
        } else if failure.starts_with("error") {
            assert!(
                matches!(&failed.failures[0].error, Error::Runtime { message,.. } if message.contains("push hook failure"))
            );
        } else {
            assert!(matches!(
                failed.failures[0].error,
                Error::Limit(Limit::FuelPerCall)
            ));
        }
        advance(&mut local, &mut sessions);
        let recovered = lua.think(&sessions[0]).expect("recovery");
        assert!(recovered.failures.is_empty(), "{:?}", recovered.failures);
        assert_eq!(
            values(lua.state.borrow().team_desires.push),
            Some([0.0, 0.5, 1.0])
        );
        assert_eq!(
            values(lua.state.borrow().team_desires.defend),
            Some([0.25, 0.0, 1.0]),
            "defend must run even on the failed push frame"
        );
    }
}

#[test]
fn malformed_team_desires_discard_only_the_first_configured_seats_orders() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Radiant, Team::Dire]);
    let seats = [
        BotSeat {
            slot: SlotId(1),
            hero: HeroId::SHADOW_FIEND,
        },
        BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        },
    ];
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &seats,
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("two seats");
    lua.add_script(SlotId(1),ScriptKind::TeamDesires,"=bad_team_desires",b"function UpdatePushLaneDesires() return {0,0.5,1} end function UpdateDefendLaneDesires() return false end").expect("team hooks");
    for seat in seats {
        lua.add_script(
            seat.slot,
            ScriptKind::Takeover,
            "=move",
            b"function Think() GetBot():Action_MoveToLocation(Vector(97,0,0)) end",
        )
        .expect("seat hook");
    }
    let turn = lua.think(&sessions[0]).expect("two-seat turn");
    assert_eq!(turn.failures.len(), 1);
    assert_eq!(
        turn.failures[0].slot,
        SlotId(1),
        "the first configured seat need not be wire slot zero"
    );
    assert!(turn.bots[0].orders.is_empty());
    assert_eq!(turn.bots[1].slot, SlotId(0));
    assert!(turn.bots[1].orders.iter().any(|order| order.order
        == bota_proto::Order::Move {
            target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(9313, 9216))
        }));
    assert_eq!(
        lua.state.borrow().team_desires,
        crate::state::TeamDesires::default()
    );
}

#[test]
fn restarted_teams_query_empty_desires_after_the_previous_interpreter_published() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut previous = team(
        "function UpdatePushLaneDesires() return {0,0.5,1} end function UpdateDefendLaneDesires() return {1,0.5,0} end",
    );
    assert!(
        previous
            .think(&sessions[0])
            .expect("published pair")
            .failures
            .is_empty()
    );
    let getter = previous.lua.global("GetPushLaneDesire").expect("getter");
    previous.lua.begin_tick();
    assert_eq!(
        previous
            .lua
            .call(getter, &[Value::Number(2.0)])
            .expect("published query"),
        vec![Value::Number(0.5)]
    );
    drop(previous);
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (_, fresh_sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let slot = SlotId(index as u8);
        let mut restarted = LuaTeam::new(
            side,
            &[BotSeat {
                slot,
                hero: HeroId::SHADOW_FIEND,
            }],
            Limits::DEFAULT,
            Keys {
                hash: [17, 19],
                random: 23,
            },
        )
        .expect("restarted team");
        restarted.add_script(slot,ScriptKind::Takeover,"=restart_reader",b"function Think() for lane=1,3 do assert(GetPushLaneDesire(lane)==0 and GetDefendLaneDesire(lane)==0) end GetBot():Action_MoveToLocation(Vector(61,0,0)) end").expect("restart reader");
        let turn = restarted
            .think(&fresh_sessions[index])
            .expect("restarted query");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(
            turn.bots[0].orders,
            vec![bota_client_core::IssuedOrder::hero(
                bota_proto::Order::Move {
                    target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(9277, 9216))
                }
            )]
        );
    }
}

#[test]
fn core_and_engine_lane_ids_select_the_same_pinned_desire() {
    assert_eq!(
        [
            bota_core::rules::LANE_MID,
            bota_core::rules::LANE_TOP,
            bota_core::rules::LANE_BOT
        ],
        [0, 1, 2]
    );
    let desires = LaneDesires::from_engine_order([0.125, 0.375, 0.625]);
    for (core, engine, expected) in [(0, 2, 0.375), (1, 1, 0.125), (2, 3, 0.625)] {
        let core_lane = Lane::from_core(core).expect("core lane");
        let engine_lane = Lane::from_engine(engine).expect("engine lane");
        assert_eq!(core_lane, engine_lane);
        assert_eq!(desires.get(core_lane), expected);
    }
    assert_eq!(Lane::from_core(3), None);
    assert_eq!(Lane::from_engine(0), None);
    assert_eq!(Lane::from_engine(4), None);
}

#[test]
fn malformed_push_retains_the_previous_pair_when_defend_returns_a_new_valid_value() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(
        "local frame=0 function UpdatePushLaneDesires() frame=frame+1 if frame==2 then return false end if frame==3 then return {1,0,0.5} end return {0.125,0.375,0.625} end function UpdateDefendLaneDesires() if frame==2 then return {0.5,0.25,0.125} end return {0.75,0.5,0.25} end",
    );
    assert!(
        lua.think(&sessions[0])
            .expect("initial pair")
            .failures
            .is_empty()
    );
    let before = lua.state.borrow().team_desires;
    advance(&mut local, &mut sessions);
    let failed = lua.think(&sessions[0]).expect("malformed push");
    assert_eq!(failed.failures.len(), 1);
    assert!(
        matches!(&failed.failures[0].error,Error::Runtime{message,..} if message.contains("team lane desires"))
    );
    assert_eq!(lua.state.borrow().team_desires, before);
    advance(&mut local, &mut sessions);
    assert!(
        lua.think(&sessions[0])
            .expect("recovered pair")
            .failures
            .is_empty()
    );
    assert_eq!(
        values(lua.state.borrow().team_desires.push),
        Some([1.0, 0.0, 0.5])
    );
}

#[test]
fn unrelated_team_think_call_fuel_exhaustion_does_not_freeze_lane_pairs() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team_with_limits(
        "local frame=0 function TeamThink() while true do end end function UpdatePushLaneDesires() frame=frame+1 return {frame*0.125,0,1} end function UpdateDefendLaneDesires() return {1,0,0.5} end",
        Limits {
            fuel_per_call: 10_000,
            fuel_per_tick: 500_000,
            ..Limits::DEFAULT
        },
    );
    for expected in [0.125, 0.25] {
        let turn = lua.think(&sessions[0]).expect("fuel-limited team hook");
        assert_eq!(turn.failures.len(), 1);
        assert!(matches!(
            turn.failures[0].error,
            Error::Limit(Limit::FuelPerCall)
        ));
        assert_eq!(
            values(lua.state.borrow().team_desires.push),
            Some([expected, 0.0, 1.0])
        );
        assert_eq!(
            values(lua.state.borrow().team_desires.defend),
            Some([1.0, 0.0, 0.5])
        );
        advance(&mut local, &mut sessions);
    }
}

#[test]
fn every_team_hook_is_attempted_after_another_team_hook_errors() {
    for failing in 1..=6 {
        let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let source = format!(
            "_G.hook_calls={{0,0,0,0,0,0}} local function mark(n) _G.hook_calls[n]=_G.hook_calls[n]+1 if n=={failing} then error('independent hook error') end end function TeamThink() mark(1) end function UpdatePushLaneDesires() mark(2) return {{0,0.5,1}} end function UpdateDefendLaneDesires() mark(3) return {{1,0.5,0}} end function UpdateFarmLaneDesires() mark(4) end function UpdateRoamDesire() mark(5) end function UpdateRoshanDesire() mark(6) end"
        );
        let mut lua = team(&source);
        let turn = lua.think(&sessions[0]).expect("team hooks");
        assert_eq!(turn.failures.len(), 1);
        assert!(
            matches!(&turn.failures[0].error,Error::Runtime{message,..} if message.contains("independent hook error"))
        );
        let Value::Table(calls) = lua.lua.global("hook_calls").expect("counters") else {
            panic!("hook counters");
        };
        for hook in 1..=6 {
            assert_eq!(
                lua.lua
                    .vm()
                    .raw_get(calls, Value::Number(f64::from(hook)))
                    .expect("counter"),
                Value::Number(1.0),
                "hook{hook} skipped after hook{failing} failed"
            );
        }
        assert_eq!(
            lua.state.borrow().team_desires.push.is_some(),
            !matches!(failing, 2 | 3)
        );
    }
}

#[test]
fn lane_getters_require_exactly_three_fuel_with_a_two_fuel_read_charge() {
    for name in ["GetPushLaneDesire", "GetDefendLaneDesire"] {
        for budget in [2, 3] {
            let mut lua = LuaTeam::new(
                Team::Radiant,
                &[BotSeat {
                    slot: SlotId(0),
                    hero: HeroId::SHADOW_FIEND,
                }],
                Limits {
                    fuel_per_tick: budget,
                    ..Limits::DEFAULT
                },
                Keys {
                    hash: [17, 19],
                    random: 23,
                },
            )
            .expect("team");
            let desires = Some(LaneDesires::from_engine_order([0.125, 0.5, 1.0]));
            lua.state.borrow_mut().team_desires = crate::state::TeamDesires {
                push: desires,
                defend: desires,
            };
            let function = lua.lua.global(name).expect("getter");
            lua.lua.begin_tick();
            let result = lua.lua.call(function, &[Value::Number(2.0)]);
            if budget == 3 {
                assert_eq!(result.expect("minimum fuel"), vec![Value::Number(0.5)]);
                assert_eq!(
                    lua.lua.tick_fuel_left(),
                    0,
                    "one top-level call fuel plus two read fuel"
                );
            } else {
                assert!(
                    matches!(result, Err(Error::Limit(Limit::FuelPerTick))),
                    "{name}: min-1 fuel must fail"
                );
            }
        }
    }
}
