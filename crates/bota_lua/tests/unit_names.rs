mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};

#[test]
fn visible_towers_report_their_engine_unit_names() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "tower_names", br#"
        function Think()
            for team=2,3 do
                local faction=team==2 and 'goodguys' or 'badguys'
                for tower=0,10 do
                    local tier=tower<9 and tower%3+1 or 4
                    local lane=tower<3 and 'top' or tower<6 and 'mid' or 'bot'
                    local suffix=tier==4 and '' or '_'..lane
                    assert(GetTower(team,tower):GetUnitName()=='npc_dota_'..faction..'_tower'..tier..suffix,'tower unit name')
                end
            end
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

#[test]
fn the_first_solo_mid_wave_reports_canonical_creep_names() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let position = sessions[0].map().creep_spawns[0][usize::from(bota_core::rules::LANE_MID)];
    for _ in 0..=sessions[0].map().pregame_ticks + 1 {
        let order = sessions[0]
            .issue(bota_client_core::IssuedOrder::hero(
                bota_proto::Order::Move {
                    target: bota_proto::Target::Pos(position),
                },
            ))
            .expect("order");
        let streams = local.step(&[&[order], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "creep_names", br#"
        function Think()
            local creeps=GetBot():GetNearbyCreeps(1600,false)
            assert(#creeps>0,'first wave is nearby')
            for _,creep in ipairs(creeps) do
                local name=creep:GetUnitName()
                assert(string.find(name,'npc_dota_creep_goodguys_',1,true)==1 or name=='npc_dota_goodguys_siege','creep unit name')
            end
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}
