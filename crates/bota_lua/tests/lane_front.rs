mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};
#[test]
fn lane_front_empty_creeps_and_tower_fronts_are_viewer_relative_on_both_sides() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (index, session) in sessions.iter().enumerate() {
        let mut lua = LuaTeam::new(
            session.team(),
            &[BotSeat {
                slot: SlotId(index as u8),
                hero: HeroId::SHADOW_FIEND,
            }],
            Limits::DEFAULT,
            Keys {
                hash: [1, 2],
                random: 3,
            },
        )
        .unwrap();
        lua.add_script(
            SlotId(index as u8),
            ScriptKind::Takeover,
            "lane_front",
            br#"function Think()
    assert(GetLaneFrontAmount(GetTeam(),LANE_MID,true)==0)
    assert(GetLaneFrontAmount(GetOpposingTeam(),LANE_MID,true)==1)
    local front=GetLaneFrontAmount(GetTeam(),LANE_MID,false)
    assert(front>0 and front<0.5)
    assert(not pcall(GetLaneFrontAmount,GetTeam(),99,true))
    assert(not pcall(GetLaneFrontAmount,GetTeam(),LANE_MID,{}))
  end"#,
        )
        .unwrap();
        let result = lua.think(session).unwrap();
        assert!(result.failures.is_empty(), "{:?}", result.failures);
    }
}

#[test]
fn observed_creeps_advance_the_front_without_waiting_for_waves() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, Order, Vec2};
    for index in 0..2 {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        let command = sessions[index]
            .issue(IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::CreateCreeps {
                    enemy: false,
                    at: Vec2::from_ints(9216, 9216),
                },
            }))
            .unwrap();
        let mut commands = [Vec::new(), Vec::new()];
        commands[index].push(command);
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&commands[0], &commands[1]]).unwrap())
        {
            support::fold(session, stream);
        }
        let mut lua = LuaTeam::new(
            sessions[index].team(),
            &[BotSeat {
                slot: SlotId(index as u8),
                hero: HeroId::SHADOW_FIEND,
            }],
            Limits::DEFAULT,
            Keys {
                hash: [1, 2],
                random: 3,
            },
        )
        .unwrap();
        lua.add_script(SlotId(index as u8),ScriptKind::Takeover,"front_wave",b"function Think() local front=GetLaneFrontAmount(GetTeam(),LANE_MID,true); assert(front>0 and front<1) end").unwrap();
        let turn = lua.think(&sessions[index]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    }
}
