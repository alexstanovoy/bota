mod support;

use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, SlotId, Target, Team, Vec2};

#[test]
fn min_and_max_compare_two_finite_engine_floats_and_reject_invalid_operands() {
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
    lua.add_script(SlotId(0), ScriptKind::Takeover, "math_queries", br#"
        function Think()
            assert(type(Min) == 'function' and type(Max) == 'function')
            assert(Min('1',2) == 1 and Max('-3',-2) == -2)
            assert(Min(0.5,0.66) == 0.5 and Min(0.66,0.5) == 0.5)
            assert(Max(-0.5,-0.66) == -0.5 and Max(-0.66,-0.5) == -0.5)
            assert(Min(-1.25,2.5) == -1.25 and Max(-1.25,2.5) == 2.5)
            assert(Min(0,0) == 0 and Max(0,0) == 0)
            assert(Min(1.25,1.25) == 1.25 and Max(1.25,1.25) == 1.25)
            local upper = 3.4028234663852886e38
            assert(Min(-upper,upper) == -upper and Max(-upper,upper) == upper)
            for _, method in ipairs({Min,Max}) do
                for _, bad in ipairs({0/0,math.huge,-math.huge,1e300,-1e300,{},false,GetBot()}) do
                    assert(not pcall(method,bad,1))
                    assert(not pcall(method,1,bad))
                end
                assert(not pcall(method))
                assert(not pcall(method,1))
                local ok, reason = pcall(method,math.huge,1)
                assert(not ok and string.find(tostring(reason),'finite number expected',1,true))
                ok, reason = pcall(method,1,1e300)
                assert(not ok and string.find(tostring(reason),'engine float within bounds expected',1,true))
            end
            GetBot():Action_MoveToLocation(Vector(123,456,0))
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
    assert_eq!(
        turn.bots[0].orders[0].order,
        Order::Move {
            target: Target::Pos(Vec2::from_ints(9339, 9672))
        }
    );
}
