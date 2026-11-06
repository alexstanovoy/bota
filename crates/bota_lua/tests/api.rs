mod support;

use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, SlotId, Team};

#[test]
fn engine_api_reads_filtered_state_and_issues_purchase_and_learning_orders() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17, 19], random: 23 }).expect("team");
    let cost = bota_core::item_by_key("item_branches").and_then(bota_core::item_def).expect("item").cost;
    let source = include_str!("scripts/api_contract.lua").replace("__ITEM_COST__", &cost.to_string());
    lua.add_script(SlotId(0), ScriptKind::Takeover, "api_contract", source.as_bytes()).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.iter().any(|issued| matches!(issued.order, Order::Buy { .. })));
    assert!(turn.bots[0].orders.iter().any(|issued| matches!(issued.order, Order::Learn { .. })));
    let orders: Vec<_> = turn.bots[0].orders.iter().map(|&issued| sessions[0].issue(issued).expect("ordinary order")).collect();
    let streams = local.step(&[&orders, &[]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) { support::fold(session, stream); }
    let hero = sessions[0].own_hero().expect("hero");
    assert!(hero.abilities.iter().any(|ability| ability.level > 0));
    assert!(hero.items.iter().any(Option::is_some));
}

#[test]
fn vectors_are_typed_immutable_and_round_trip_through_an_order() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17, 19], random: 23 }).expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "vector_contract", br#"
        function Think()
            local v = Vector(3,4,0)
            assert(v:Length() == 5 and v:Length2D() == 5)
            assert(v:Dot(Vector(1,2,3)) == 11)
            assert(v:Cross(Vector(0,0,1)) == Vector(4,-3,0))
            assert(v + Vector(1,2,3) == Vector(4,6,3))
            assert(2*v == Vector(6,8,0) and v/2 == Vector(1.5,2,0))
            assert(-v == Vector(-3,-4,0))
            assert(v:Lerp(Vector(5,6,0),0.5) == Vector(4,5,0))
            assert(math.abs(v:Normalized():Length()-1) < 1e-12)
            assert(not pcall(function() v.x = 7 end))
            assert(not pcall(Vector,0/0,1,2))
            assert(not pcall(Vector,math.huge,1,2))
            assert(not pcall(function() return v/0 end))
            GetBot():Action_MoveToLocation(GetBot():GetLocation())
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders[0].order, Order::Move { target: bota_proto::Target::Pos(sessions[0].own_hero().expect("hero").pos) });
}

#[test]
fn visible_queries_and_casts_use_the_filtered_session_and_real_order_path() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17, 19], random: 23 }).expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "query_cast_contract", br#"
        local turn = 0
        function Think()
            local bot = GetBot()
            assert(type(bot:IsInvulnerable()) == 'boolean')
            assert(type(bot:IsMagicImmune()) == 'boolean')
            assert(not bot:HasScepter())
            assert(not bot:IsChanneling() and not bot:IsUsingAbility() or turn > 1)
            assert(bot:DistanceFromFountain() >= 0)
            assert(type(bot:GetNearbyHeroes(1600,true,BOT_MODE_NONE)) == 'table')
            assert(type(bot:GetNearbyCreeps(1600,false)) == 'table')
            assert(type(bot:GetNearbyTowers(1600,false)) == 'table')
            assert(GetTower(TEAM_DIRE,3) ~= nil)
            assert(not bot:WasRecentlyDamagedByHero(bot,1))
            local p = GetLocationAlongLane(LANE_MID,0.5)
            assert(p.x == p.x and p.y == p.y)
            local area = bot:FindAoELocation(true,true,bot:GetLocation(),700,250,0,0)
            assert(area.count == 0 and area.targetloc.x == area.targetloc.x)
            local ability = bot:GetAbilityByName('nevermore_shadowraze1')
            assert(ability:GetSpecialValueInt('shadowraze_radius') > 0)
            assert(ability:GetAbilityDamage() > 0)
            turn = turn + 1
            if turn == 1 then bot:ActionImmediate_LevelAbility(ability:GetName())
            elseif turn == 2 then bot:Action_UseAbility(ability) end
        end
    "#).expect("script");
    let before = sessions[0].own_hero().expect("hero").mana;
    let mut cast = false;
    for _ in 0..30 {
        let turn = lua.think(&sessions[0]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        cast |= turn.bots[0].orders.iter().any(|issued| matches!(issued.order, Order::Cast { .. }));
        let orders: Vec<_> = turn.bots[0].orders.iter().map(|&issued| sessions[0].issue(issued).expect("ordinary order")).collect();
        let streams = local.step(&[&orders, &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) { support::fold(session, stream); }
    }
    assert!(cast);
    assert!(sessions[0].own_hero().expect("hero").mana < before);
}
