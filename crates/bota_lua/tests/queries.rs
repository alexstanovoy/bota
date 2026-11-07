mod support;

use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team, Order, Target, Vec2};

#[test]
fn creep_aoe_search_excludes_allied_buildings_during_a_solo_mid_tick_loop() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17, 19], random: 23 }).expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "creep_area", br#"
        function Think()
            local b=GetBot()
            local area=b:FindAoELocation(false,false,b:GetLocation(),0,1600,0,0)
            assert(area.count==0, 'a creep search must not count buildings')
            b:Action_MoveToLocation(b:GetLocation())
        end
    "#).expect("script");
    for _ in 0..30 {
        let turn = lua.think(&sessions[0]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        let orders: Vec<_> = turn.bots[0].orders.iter().map(|&issued| sessions[0].issue(issued).expect("ordinary order")).collect();
        let streams = local.step(&[&orders, &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) { support::fold(session, stream); }
    }
}

fn a_session_with_a_visible_moving_opponent() -> bota_client_core::ClientSession {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let opponent = sessions[1].own_player().expect("player").unit.expect("hero");
    for _ in 0..2000 {
        let order = bota_client_core::IssuedOrder::hero(Order::Move { target: Target::Pos(Vec2::from_ints(9216,9216)) });
        let radiant = sessions[0].issue(order).expect("order");
        let dire = sessions[1].issue(order).expect("order");
        let streams = local.step(&[&[radiant], &[dire]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) { support::fold(session, stream); }
        if sessions[0].entities().iter().any(|track| track.id == opponent && track.visible && track.velocity.is_some_and(|velocity| velocity.delta != Vec2::ZERO)) {
            return sessions.remove(0);
        }
    }
    panic!("the two heroes never saw each other moving");
}

#[test]
#[allow(clippy::float_arithmetic, reason = "compare an observed finite displacement with the engine coordinate projection")]
fn aoe_future_delay_projects_only_the_observed_enemy_motion() {
    let session = a_session_with_a_visible_moving_opponent();
    let track = session.entities().iter().find(|track| track.visible && track.unit.kind == bota_proto::UnitKind::Hero && track.unit.team == Team::Dire).expect("opponent");
    let velocity = track.velocity.expect("observed motion");
    let coordinate = |position: bota_proto::Fixed, delta: bota_proto::Fixed| (f64::from(position.raw) + f64::from(delta.raw) * 15.0 / f64::from(velocity.elapsed_ticks)) / 65536.0 - 9216.0;
    let source = format!("function Think() local b=GetBot() local a=b:FindAoELocation(true,true,b:GetLocation(),1600,0,0.5,0) assert(a.count==1) assert((a.targetloc-Vector({},{},0)):Length2D()<1,'future location must use observed movement') end", coordinate(track.unit.pos.x,velocity.delta.x), coordinate(track.unit.pos.y,velocity.delta.y));
    let mut lua = LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17,19], random:23 }).expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "future_area", source.as_bytes()).expect("script");
    let turn = lua.think(&session).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

#[test]
fn nearby_enemy_hero_queries_ignore_the_allied_mode_filter() {
    let session = a_session_with_a_visible_moving_opponent();
    let mut lua = LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17,19], random:23 }).expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "enemy_modes", b"function Think() local b=GetBot() assert(#b:GetNearbyHeroes(1600,true,99)==1,'enemy mode is not known or filtered') end").expect("script");
    let turn = lua.think(&session).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}
