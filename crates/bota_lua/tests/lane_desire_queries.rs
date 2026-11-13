mod support;

use bota_client_core::ClientSession;
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, SlotId, Target, Team, Vec2};
use bota_server::local::LocalMatch;

fn team(side: Team, slots: &[SlotId]) -> LuaTeam {
    let seats: Vec<_> = slots
        .iter()
        .map(|&slot| BotSeat {
            slot,
            hero: HeroId::SHADOW_FIEND,
        })
        .collect();
    let mut lua = LuaTeam::new(
        side,
        &seats,
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.set_gc_stress(GcStress::Full);
    lua
}

fn advance(local: &mut LocalMatch, sessions: &mut [ClientSession]) {
    let empty: Vec<&[bota_proto::OutgoingOrder]> = vec![&[]; sessions.len()];
    let streams = local.step(&empty).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
}

fn assert_move(turn: &bota_lua::TeamTurn, slot: usize, x: i32) {
    assert_eq!(
        turn.bots[slot].orders.len(),
        1,
        "the getter hook reached its final action"
    );
    assert_eq!(
        turn.bots[slot].orders[0].order,
        Order::Move {
            target: Target::Pos(Vec2::from_ints(9216 + x, 9416))
        }
    );
}

#[test]
fn lane_desire_queries_map_engine_lanes_and_reset_nil_values_without_crossing_sides() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut teams = [
        team(Team::Radiant, &[SlotId(0)]),
        team(Team::Dire, &[SlotId(1)]),
    ];
    for (index, lua) in teams.iter_mut().enumerate() {
        let slot = SlotId(index as u8);
        lua.add_script(
            slot,
            ScriptKind::TeamDesires,
            "lane_values",
            br#"
            local frame = 0
            function UpdatePushLaneDesires()
                frame = frame + 1
                if frame == 2 then return nil end
                if frame == 3 then
                    if GetTeam() == TEAM_RADIANT then return {0.125,0.375,-0.0} end
                    return {0.875,0.625,-0.0}
                end
                if GetTeam() == TEAM_RADIANT then return {0.125,0.375,0.625} end
                return {0.875,0.625,0.375}
            end
            function UpdateDefendLaneDesires()
                if frame == 3 then return nil end
                if GetTeam() == TEAM_RADIANT then return {0.75,0.5,0.25} end
                return {0.25,0.5,0.75}
            end
        "#,
        )
        .expect("producer script");
        lua.add_script(slot, ScriptKind::Takeover, "lane_getters", br#"
            local frame = 0
            function Think()
                frame = frame + 1
                assert(LANE_TOP == 1 and LANE_MID == 2 and LANE_BOT == 3)
                local push, defend
                if GetTeam() == TEAM_RADIANT then push={0.125,0.375,0.625}; defend={0.75,0.5,0.25}
                else push={0.875,0.625,0.375}; defend={0.25,0.5,0.75} end
                if frame == 3 then push[3] = 0; assert(1/GetPushLaneDesire(LANE_BOT) == math.huge) end
                for lane=1,3 do
                    assert(GetPushLaneDesire(lane) == (frame == 2 and 0 or push[lane]))
                    assert(GetDefendLaneDesire(lane) == (frame == 3 and 0 or defend[lane]))
                end
                for _, lane in ipairs({-1,0,4,0.5,1e300,-1e300,0/0,math.huge,-math.huge,{}}) do
                    assert(not pcall(GetPushLaneDesire, lane))
                    assert(not pcall(GetDefendLaneDesire, lane))
                end
                local ok, reason = pcall(GetPushLaneDesire, 0)
                assert(not ok and string.find(tostring(reason), 'integer within bounds expected', 1, true))
                GetBot():Action_MoveToLocation(Vector(100+frame,200,0))
            end
        "#).expect("getter script");
    }
    for frame in 1..=3 {
        for (index, lua) in teams.iter_mut().enumerate() {
            let turn = lua.think(&sessions[index]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            assert_move(&turn, 0, 100 + frame);
        }
        advance(&mut local, &mut sessions);
    }
}

#[test]
fn lane_desire_queries_keep_previous_pairs_on_lane_failure_and_publish_before_unrelated_errors() {
    for (side, slots, session) in [
        (Team::Radiant, [SlotId(2), SlotId(0)], 0),
        (Team::Dire, [SlotId(3), SlotId(1)], 1),
    ] {
        let (mut local, mut sessions) =
            support::local(&[Team::Radiant, Team::Dire, Team::Radiant, Team::Dire]);
        let mut lua = team(side, &slots);
        lua.add_script(
            slots[0],
            ScriptKind::TeamDesires,
            "lane_pair_errors",
            br#"
            local frame, defended = 0, 0
            function TeamThink() error('unrelated team error') end
            function UpdatePushLaneDesires()
                frame = frame + 1
                if frame == 2 then error('push failed') end
                return {0.125*frame,0.25*frame,0.03125*frame}
            end
            function UpdateDefendLaneDesires()
                defended = defended + 1
                assert(defended == frame, 'both lane hooks are attempted')
                return {0.0625*frame,0.125*frame,0.25*frame}
            end
        "#,
        )
        .expect("producer script");
        for slot in slots {
            lua.add_script(
                slot,
                ScriptKind::Takeover,
                "cached_lane_getters",
                br#"
                local frame = 0
                function Think()
                    frame = frame + 1
                    local published = frame == 2 and 1 or frame
                    assert(GetPushLaneDesire(LANE_TOP) == 0.125*published)
                    assert(GetPushLaneDesire(LANE_MID) == 0.25*published)
                    assert(GetPushLaneDesire(LANE_BOT) == 0.03125*published)
                    assert(GetDefendLaneDesire(LANE_TOP) == 0.0625*published)
                    assert(GetDefendLaneDesire(LANE_MID) == 0.125*published)
                    assert(GetDefendLaneDesire(LANE_BOT) == 0.25*published)
                    GetBot():Action_MoveToLocation(Vector(100+frame,200,0))
                end
            "#,
            )
            .expect("getter script");
        }
        for frame in 1..=3 {
            let turn = lua.think(&sessions[session]).expect("think");
            assert_eq!(turn.failures.len(), 1);
            assert_eq!(
                turn.failures[0].slot, slots[0],
                "the first configured seat owns the team error"
            );
            assert!(
                turn.failures[0]
                    .error
                    .to_string()
                    .contains("unrelated team error")
            );
            assert!(turn.bots[0].orders.is_empty());
            assert_eq!(turn.bots[1].slot, slots[1]);
            assert_move(&turn, 1, 100 + frame);
            advance(&mut local, &mut sessions);
        }
    }
}

#[test]
fn lane_desire_queries_without_script_overrides_use_zero_baselines() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Team::Radiant, &[SlotId(0)]);
    lua.add_script(SlotId(0), ScriptKind::Takeover, "native_lane_baseline", br#"
        function Think()
            for lane=1,3 do assert(GetPushLaneDesire(lane) == 0 and GetDefendLaneDesire(lane) == 0) end
            GetBot():Action_MoveToLocation(Vector(101,200,0))
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_move(&turn, 0, 101);
}
