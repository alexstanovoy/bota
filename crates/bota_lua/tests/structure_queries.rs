mod support;

use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, ServerMsg, SlotId, Target, Team, Vec2};

fn positions(map: &bota_core::MapDef, side: usize) -> String {
    use bota_core::rules::{LANE_BOT, LANE_MID, LANE_TOP};
    // The API's top/mid/bot order differs from the core catalogue's mid/top/bot.
    let mut values = Vec::new();
    for (lane, ranged) in [
        (LANE_TOP, false),
        (LANE_TOP, true),
        (LANE_MID, false),
        (LANE_MID, true),
        (LANE_BOT, false),
        (LANE_BOT, true),
    ] {
        let (_, _, at) = map.barracks[side]
            .iter()
            .find(|&&(l, r, _)| l == lane && r == ranged)
            .expect("barracks");
        values.push(format!(
            "{{{},{}}}",
            at.x.to_int() - 9216,
            at.y.to_int() - 9216
        ));
    }
    format!("{{{}}}", values.join(","))
}

#[test]
fn structure_queries_map_each_barracks_and_ancient_from_both_filtered_views() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut teams = Vec::new();
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let slot = SlotId(index as u8);
        let mut lua = LuaTeam::new(
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
        .expect("team");
        lua.set_gc_stress(GcStress::Full);
        let map = sessions[index].map();
        let ancient = map.ancients[index].expect("ancient");
        let source = r#"
            local phase, enemy = 0, nil
            local own_positions = __OWN__
            local enemy_positions = __ENEMY__
            local function check(unit, at)
                assert(unit and unit:CanBeSeen())
                local pos = unit:GetLocation()
                assert(pos.x == at[1] and pos.y == at[2])
            end
            function Think()
                phase = phase + 1
                assert(type(GetBarracks) == 'function' and type(GetAncient) == 'function')
                for index=0,5 do
                    check(GetBarracks(GetTeam(),index), own_positions[index+1])
                    local other = GetBarracks(GetOpposingTeam(),index)
                    if phase ~= 2 then check(other, enemy_positions[index+1])
                    else assert(other == nil) end
                end
                local ancient = GetAncient(GetTeam())
                check(ancient, {__AX__,__AY__})
                if phase ~= 2 then
                    local current = GetAncient(GetOpposingTeam())
                    assert(current and current:CanBeSeen())
                    if phase == 3 then assert(current == enemy) else enemy = current end
                else
                    assert(GetAncient(GetOpposingTeam()) == nil)
                    assert(enemy and not enemy:CanBeSeen())
                end
                assert(GetBarracks(GetTeam(),6) == nil)
                assert(GetBarracks(GetTeam(),2147483647) == nil)
                for _, index in ipairs({-1,0.5,1e300,-1e300,0/0,math.huge,-math.huge,{}}) do
                    assert(not pcall(GetBarracks,GetTeam(),index))
                end
                for _, side in ipairs({-1,0,1,4,2.5,1e300,-1e300,0/0,math.huge,-math.huge,{}}) do
                    assert(not pcall(GetBarracks,side,0))
                    assert(not pcall(GetAncient,side))
                end
                local ok, reason = pcall(GetBarracks,GetTeam(),math.huge)
                assert(not ok and string.find(tostring(reason),'finite number expected',1,true))
                GetBot():Action_MoveToLocation(Vector(100+phase,200,0))
            end
        "#
        .replace("__OWN__", &positions(map, index))
        .replace("__ENEMY__", &positions(map, 1 - index))
        .replace("__AX__", &(ancient.x.to_int() - 9216).to_string())
        .replace("__AY__", &(ancient.y.to_int() - 9216).to_string());
        lua.add_script(
            slot,
            ScriptKind::Takeover,
            "structure_queries",
            source.as_bytes(),
        )
        .expect("script");
        teams.push(lua);
    }
    for phase in 1..=3 {
        for (index, lua) in teams.iter_mut().enumerate() {
            let turn = lua.think(&sessions[index]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            assert_eq!(turn.bots[0].orders.len(), 1);
            assert_eq!(
                turn.bots[0].orders[0].order,
                Order::Move {
                    target: Target::Pos(Vec2::from_ints(9316 + phase, 9416))
                }
            );
        }
        if phase < 3 {
            let streams = local.step(&[&[], &[]]).expect("received buildings");
            for (session, stream) in sessions.iter_mut().zip(streams) {
                let enemy = session.enemy();
                let stream = stream.into_iter().map(|message| match message {
                    // Buildings are currently always shown by the simulation.
                    // Exercise the API with a received stream withholding these facts.
                    ServerMsg::Snapshot { mut view } if phase == 1 => {
                        view.units.retain(|unit| {
                            unit.team != enemy || !bota_core::kind_def(unit.kind).structure
                        });
                        view.aggro.clear();
                        ServerMsg::Snapshot { view }
                    }
                    other => other,
                });
                support::fold(session, stream);
            }
        }
    }
}
