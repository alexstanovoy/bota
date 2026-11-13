mod support;
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, HeroId, Order, ServerMsg, SlotId, Target, Team, Vec2};

#[test]
fn distance_queries_use_engine_xy_and_reject_forged_or_forgotten_units() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let positions = [Vec2::from_ints(9000, 9000), Vec2::from_ints(9300, 9400)];
    {
        let world = local.world_mut();
        world.cheats = true;
        for (index, &position) in positions.iter().enumerate() {
            let unit = world.seats[index].unit.expect("hero");
            let cheat = Cheat::Teleport { at: position };
            world
                .check_cheat(SlotId(index as u8), Some(unit), &cheat)
                .expect("valid teleport");
            world.cheat(SlotId(index as u8), Some(unit), cheat, &mut Vec::new());
        }
        world.cheat(
            SlotId(0),
            None,
            Cheat::AllVision { on: true },
            &mut Vec::new(),
        );
    }
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[], &[]]).expect("positions"))
    {
        support::fold(session, stream);
    }
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        assert_eq!(
            sessions[index].own_hero().expect("hero").pos,
            positions[index]
        );
        let slot = SlotId(index as u8);
        let enemy = sessions[index]
            .current()
            .expect("view")
            .units
            .iter()
            .find(|unit| unit.hero.is_some() && unit.team == side.enemy())
            .expect("visible enemy")
            .id;
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
        let source=r#"
            local phase,enemy=0,nil
            function Think()
                phase=phase+1
                local bot=GetBot()
                if phase==1 then enemy=GetUnitList(UNIT_LIST_ENEMY_HEROES)[1] end
                local point=Vector(__X__,__Y__,9000)
                assert(GetUnitToLocationDistance(bot,point)==5 and GetUnitToLocationDistanceSqr(bot,point)==25)
                if phase<3 then
                    assert(GetUnitToUnitDistanceSqr(bot,enemy)==250000)
                    assert(GetUnitToLocationDistance(bot,enemy:GetLocation())==500)
                    if phase==2 then assert(not enemy:CanBeSeen()) end
                else
                    local ok,reason=pcall(GetUnitToUnitDistanceSqr,bot,enemy)
                    assert(not ok and string.find(tostring(reason),'unit is no longer known',1,true))
                end
                for _,method in ipairs({GetUnitToLocationDistance,GetUnitToLocationDistanceSqr}) do
                    assert(not pcall(method,{},point) and not pcall(method,bot,{x=0,y=0,z=0}))
                    assert(not pcall(method,bot,Vector(1e300,0,0)))
                    assert(not pcall(function() method(bot,Vector(0,0/0,0)) end))
                end
                assert(not pcall(GetUnitToUnitDistanceSqr,bot,{}) and not pcall(GetUnitToUnitDistanceSqr,{},bot))
                bot:Action_MoveToLocation(Vector(phase,200,0))
            end
        "#.replace("__X__",&(positions[index].x.to_int()-9216+3).to_string()).replace("__Y__",&(positions[index].y.to_int()-9216+4).to_string());
        lua.add_script(
            slot,
            ScriptKind::Takeover,
            "distance_queries",
            source.as_bytes(),
        )
        .expect("script");
        for phase in 1..=3 {
            let turn = lua.think(&sessions[index]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            assert_eq!(
                turn.bots[0].orders[0].order,
                Order::Move {
                    target: Target::Pos(Vec2::from_ints(9216 + phase, 9416))
                }
            );
            assert_eq!(turn.bots[0].orders.len(), 1);
            if phase < 3 {
                let mut view = sessions[index].current().expect("view").clone();
                view.tick += if phase == 1 { 1 } else { 481 };
                view.units.retain(|unit| unit.id != enemy);
                view.aggro.clear();
                view.projectiles.clear();
                let tick = view.tick;
                sessions[index]
                    .ingest(ServerMsg::Snapshot { view }, &mut ())
                    .expect("filtered view");
                sessions[index]
                    .ingest(
                        ServerMsg::Events {
                            tick,
                            events: Vec::new(),
                        },
                        &mut (),
                    )
                    .expect("events");
            }
        }
    }
}
