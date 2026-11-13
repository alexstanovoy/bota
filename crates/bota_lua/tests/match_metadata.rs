mod support;

use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, SlotId, Target, Team, Vec2};

#[test]
fn solo_mid_reports_its_engine_mode_and_assigned_mid_lane_on_both_sides() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
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
        lua.add_script(
            slot,
            ScriptKind::Takeover,
            "match_metadata",
            br#"
            function Think()
                assert(type(GetGameMode) == 'function' and GetGameMode() == 21)
                local bot = GetBot()
                assert(bot:GetAssignedLane() == 2 and LANE_MID == 2)
                local tower = GetTower(GetTeam(), TOWER_MID_1)
                assert(tower and tower:GetAssignedLane() == 0)
                local method = bot.GetAssignedLane
                local ok, reason = pcall(method, {})
                assert(not ok and string.find(tostring(reason), 'unit handle expected', 1, true))
                assert(not pcall(method, setmetatable({}, {__index=bot})))
                bot:Action_MoveToLocation(Vector(123,456,0))
            end
        "#,
        )
        .expect("script");
        let turn = lua.think(&sessions[index]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(turn.bots[0].orders.len(), 1);
        assert_eq!(
            turn.bots[0].orders[0].order,
            Order::Move {
                target: Target::Pos(Vec2::from_ints(9339, 9672))
            }
        );
    }
}
