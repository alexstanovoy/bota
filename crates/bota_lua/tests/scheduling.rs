mod support;

use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, SlotId, Target, Team};

fn team(limits: Limits) -> LuaTeam {
    LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        limits,
        Keys {
            hash: [1, 2],
            random: 3,
        },
    )
    .expect("team")
}

#[test]
fn modes_arbitrate_every_nine_ticks_and_run_transition_hooks() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Limits::DEFAULT);
    lua.add_script(
        SlotId(0),
        ScriptKind::Mode(1),
        "low",
        br#"
        local started = false
        function GetDesire() return 0.4 end
        function OnStart() assert(GetBot().ended == 2) started = true end
        function Think() assert(started) GetBot():Action_MoveToLocation(Vector(1,0,0)) end
    "#,
    )
    .expect("mode");
    lua.add_script(
        SlotId(0),
        ScriptKind::Mode(2),
        "high",
        br#"
        local started = false
        function GetDesire() if GameTime() < 0.3 then return 0.8 else return 0.2 end end
        function OnStart() started = true end
        function OnEnd() GetBot().ended = 2 end
        function Think() assert(started) GetBot():Action_MoveToLocation(Vector(2,0,0)) end
    "#,
    )
    .expect("mode");
    for tick in 0..10 {
        let turn = lua.think(&sessions[0]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(turn.bots[0].orders,vec![
            bota_client_core::IssuedOrder::hero(Order::Move {target:Target::Pos(bota_proto::Vec2::from_ints(9216+if tick<9 {2}else{1},9216))}),
            bota_client_core::IssuedOrder::hero(Order::Learn {slot:bota_proto::AbilitySlot(0)}),
            bota_client_core::IssuedOrder::hero(Order::Buy {item:bota_proto::ItemId::IRON_BRANCH}),
        ],"mode overrides replace body movement while missing auxiliary hooks retain their defaults");
        let Order::Move {
            target: Target::Pos(position),
        } = turn.bots[0].orders[0].order
        else {
            panic!("move")
        };
        assert_eq!(position.x.to_int(), 9216 + if tick < 9 { 2 } else { 1 });
        let streams = local.step(&[&[], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
}

#[test]
fn takeover_runs_team_and_auxiliary_hooks_without_running_modes() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Limits::DEFAULT);
    lua.add_script(SlotId(0), ScriptKind::Takeover, "takeover", b"function Think() assert(_G.team_turns == 1) GetBot().hooks = 0 GetBot():Action_MoveToLocation(Vector(3,0,0)) end").expect("script");
    lua.add_script(SlotId(0), ScriptKind::Mode(1), "unused_mode", b"function GetDesire() error('takeover suppresses modes') end function Think() error('takeover suppresses modes') end").expect("script");
    lua.add_script(SlotId(0), ScriptKind::ItemPurchase, "purchase", b"function ItemPurchaseThink() assert(GetBot().hooks == 5) GetBot():Action_MoveToLocation(Vector(5,0,0)) end").expect("script");
    lua.add_script(SlotId(0), ScriptKind::AbilityItemUsage, "abilities", br#"
        local function hook() GetBot().hooks = GetBot().hooks + 1 end
        AbilityUsageThink = hook ItemUsageThink = hook CourierUsageThink = hook BuybackUsageThink = hook AbilityLevelUpThink = hook
    "#).expect("script");
    lua.add_script(
        SlotId(0),
        ScriptKind::TeamDesires,
        "team",
        b"function TeamThink() _G.team_turns = (_G.team_turns or 0) + 1 end",
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 2);
}

#[test]
fn sparse_fuel_failures_warn_at_most_once_per_thirty_ticks() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Limits {
        fuel_per_call: 500,
        fuel_per_tick: 10_000,
        ..Limits::DEFAULT
    });
    lua.add_script(SlotId(0), ScriptKind::Takeover, "sparse_spin", b"local calls = 0 function Think() calls = calls + 1 if calls%20 == 1 then while true do end end end").expect("script");
    let mut warnings = Vec::new();
    for tick in 0..41 {
        let turn = lua.think(&sessions[0]).expect("think");
        if turn.failures.iter().any(|failure| failure.report) {
            warnings.push(tick);
        }
        let streams = local.step(&[&[], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
    assert_eq!(warnings, [0, 40]);
}

#[test]
fn a_seat_cannot_install_two_takeovers_for_the_same_override() {
    let mut lua = team(Limits::DEFAULT);
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "first",
        b"function Think() end",
    )
    .expect("script");
    let error = lua
        .add_script(
            SlotId(0),
            ScriptKind::Takeover,
            "second",
            b"function Think() end",
        )
        .expect_err("one takeover per seat");
    assert!(error.to_string().contains("duplicate script override"));
}
