mod support;

use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, ServerMsg, SlotId, Target, Team, Vec2};

fn destination(order: Order) -> Vec2 {
    let Order::Move { target: Target::Pos(position) } = order else { panic!("query-derived move expected") };
    position
}

#[test]
fn structure_queries_return_received_barracks_and_ancients_from_both_sides() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut teams = Vec::new();
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let slot = SlotId(index as u8);
        let mut lua = LuaTeam::new(side, &[BotSeat { slot, hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17,19], random: 23 }).expect("team");
        lua.set_gc_stress(GcStress::Full);
        lua.add_script(slot, ScriptKind::Takeover,"structure_queries",br#"
            local frame, remembered = 0, nil
            function Think()
                frame = frame + 1
                local phase, index = math.floor((frame-1)/6)+1, (frame-1)%6
                local own = GetBarracks(GetTeam(),index)
                assert(own and own:CanBeSeen())
                GetBot():Action_MoveToLocation(own:GetLocation())
                local ancient = GetAncient(GetTeam())
                assert(ancient and ancient:CanBeSeen())
                GetBot():Action_MoveToLocation(ancient:GetLocation())
                local other = GetBarracks(GetOpposingTeam(),index)
                if phase == 2 then
                    assert(other == nil and GetAncient(GetOpposingTeam()) == nil)
                    assert(remembered and not remembered:CanBeSeen())
                else
                    assert(other and other:CanBeSeen())
                    GetBot():Action_MoveToLocation(other:GetLocation())
                    local enemy = GetAncient(GetOpposingTeam())
                    assert(enemy and enemy:CanBeSeen())
                    if phase == 1 then remembered = enemy else assert(enemy == remembered) end
                end
                if frame == 1 then
                    assert(GetBarracks(GetTeam(),'0') == GetBarracks(GetTeam(),0))
                    assert(GetBarracks(GetTeam(),6) == nil and GetBarracks(GetTeam(),2147483647) == nil)
                    for _, index in ipairs({-1,0.5,1e300,-1e300,0/0,math.huge,-math.huge,{}}) do assert(not pcall(GetBarracks,GetTeam(),index)) end
                    for _, side in ipairs({-1,0,1,4,2.5,1e300,-1e300,0/0,math.huge,-math.huge,{}}) do
                        assert(not pcall(GetBarracks,side,0) and not pcall(GetAncient,side))
                    end
                    local ok, reason = pcall(GetBarracks,GetTeam(),math.huge)
                    assert(not ok and string.find(tostring(reason),'finite number expected',1,true))
                end
                GetBot():Action_MoveToLocation(Vector(100+frame,200,0))
            end
        "#).expect("script");
        teams.push(lua);
    }
    let mut observed: [[Vec<Vec2>;3];2] = std::array::from_fn(|_|std::array::from_fn(|_|Vec::new()));
    let mut enemy_observed: [[Vec<Vec2>;3];2] = std::array::from_fn(|_|std::array::from_fn(|_|Vec::new()));
    for frame in 1..=18 {
        let phase = (frame-1)/6;
        for (index,lua) in teams.iter_mut().enumerate() {
            let turn = lua.think(&sessions[index]).expect("think");
            assert!(turn.failures.is_empty(), "frame {frame}: {:?}",turn.failures);
            let orders = &turn.bots[0].orders;
            assert_eq!(orders.len(),if phase == 1 {3} else {4});
            observed[index][phase].push(destination(orders[0].order));
            assert_eq!(destination(orders[1].order),sessions[index].map().ancients[index].expect("ancient"));
            if phase != 1 { enemy_observed[index][phase].push(destination(orders[2].order)); }
            assert_eq!(destination(orders.last().expect("final action").order),Vec2::from_ints(9316+frame as i32,9416));
        }
        if frame < 18 {
            let streams = local.step(&[&[],&[]]).expect("received buildings");
            for (session,stream) in sessions.iter_mut().zip(streams) {
                let enemy = session.enemy();
                support::fold(session,stream.into_iter().map(|message| match message {
                    // The producer always shows buildings. Independently exercise withholding at the API boundary.
                    ServerMsg::Snapshot { mut view } if (6..12).contains(&frame) => {
                        view.units.retain(|unit|unit.team != enemy || !bota_core::kind_def(unit.kind).structure);
                        view.aggro.clear();
                        ServerMsg::Snapshot { view }
                    }
                    other => other,
                }));
            }
        }
    }
    // This pins membership against catalogue geometry, not the unsourced engine index order.
    for index in 0..2 {
        let mut expected: Vec<_> = sessions[index].map().barracks[index].iter().map(|&(_,_,pos)|pos).collect();
        expected.sort_unstable();
        let mut enemy: Vec<_> = sessions[index].map().barracks[1-index].iter().map(|&(_,_,pos)|pos).collect();
        enemy.sort_unstable();
        for phase in 0..3 {
            observed[index][phase].sort_unstable();
            assert_eq!(observed[index][phase],expected);
            if phase != 1 {
                enemy_observed[index][phase].sort_unstable();
                assert_eq!(enemy_observed[index][phase],enemy);
            }
        }
    }
}
