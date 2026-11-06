mod support;

use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};

fn team() -> LuaTeam {
    LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17, 19], random: 23 }).expect("team")
}

#[test]
fn copied_tables_and_bad_numeric_arguments_cannot_forge_engine_objects() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team();
    lua.add_script(SlotId(0), ScriptKind::Takeover, "hostile_arguments", br#"
        function Think()
            local bot = GetBot()
            local ability = bot:GetAbilityByName('nevermore_shadowraze1')
            local forged = {}
            for key,value in pairs(bot) do forged[key] = value end
            assert(not pcall(bot.IsAlive,forged))
            assert(not pcall(bot.Action_MoveToLocation,forged,Vector(0,0,0)))
            assert(not pcall(bot.SetTarget,bot,forged))
            assert(not pcall(ability.GetName,forged))
            assert(not pcall(bot.Action_UseAbility,bot,forged))
            assert(not pcall(bot.Action_MoveToLocation,bot,{x=0,y=0,z=0}))
            for _,number in ipairs({0/0,math.huge,-math.huge}) do
                assert(not pcall(bot.GetItemInSlot,bot,number))
                assert(not pcall(bot.GetNearbyHeroes,bot,number,true,BOT_MODE_NONE))
                assert(not pcall(GetLocationAlongLane,number,0.5))
                assert(not pcall(bot.SetNextItemPurchaseValue,bot,number))
            end
            assert(GetTeamMember(0) == nil)
            assert(GetLocationAlongLane ~= nil)
            assert(not pcall(GetLocationAlongLane,-1e308,0.5))
            assert(bot:GetItemInSlot(-1e308) == nil)
            assert(bot:GetItemInSlot(1e308) == nil)
            assert(getmetatable(Vector(0,0,0)) == false)
            assert(not pcall(function() return Vector(1,2,3) + 1 end))
            assert(not pcall(function() return 1 - Vector(1,2,3) end))
            local normalized = Vector(1.7e308,1.7e308,0):Normalized()
            assert(math.abs(normalized:Length()-1) < 1e-12)
            bot:Action_MoveToLocation(Vector(0,0,0))
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
}

#[test]
fn cached_unit_and_ability_handles_survive_both_gc_stress_modes() {
    for stress in [GcStress::Full, GcStress::Incremental] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let mut lua = team();
        lua.add_script(SlotId(0), ScriptKind::Takeover, "gc_roots", br#"
            local bot = GetBot()
            local ability = bot:GetAbilityByName('nevermore_shadowraze1')
            bot.saved = {value = 17}
            function Think()
                collectgarbage()
                assert(bot == GetBot() and bot.saved.value == 17)
                assert(ability == bot:GetAbilityByName('nevermore_shadowraze1'))
                assert(ability:GetName() == 'nevermore_shadowraze1')
                assert(bot:GetHealth() > 0)
                bot:Action_MoveToLocation(bot:GetLocation())
            end
        "#).expect("script");
        lua.set_gc_stress(stress);
        for _ in 0..3 {
            let turn = lua.think(&sessions[0]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            let streams = local.step(&[&[], &[]]).expect("step");
            for (session, stream) in sessions.iter_mut().zip(streams) { support::fold(session, stream); }
        }
    }
}

#[test]
fn repeating_a_snapshot_cannot_refill_the_team_fuel_budget() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team();
    lua.add_script(SlotId(0), ScriptKind::Takeover, "spin", b"function Think() while true do end end").expect("script");
    let turn = lua.think(&sessions[0]).expect("first tick");
    assert_eq!(turn.failures.len(), 1);
    let error = lua.think(&sessions[0]).expect_err("a tick is consumed only once");
    assert!(error.to_string().contains("increasing tick"));
}
