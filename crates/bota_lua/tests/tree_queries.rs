mod support;
use bota_lua::{BotSeat,Keys,Limits,LuaTeam,ScriptKind};
use bota_client_core::IssuedOrder;
use bota_proto::{Cheat,HeroId,Order,SlotId,Team};
#[test] fn nearby_trees_use_real_forest_ids_and_do_not_query_a_foreign_body_in_fog() {
 for index in 0..2 {
    let (mut local,mut sessions)=support::local_with_cheats(&[Team::Radiant,Team::Dire],bota_core::MAP2_ID);
    let tree=sessions[0].statics().trees()[0];
    let commands=sessions.iter_mut().map(|session|vec![session.issue(IssuedOrder::hero(Order::Cheat{cheat:Cheat::Teleport {at:tree}})).unwrap()]).collect::<Vec<_>>();
    for (session,stream) in sessions.iter_mut().zip(local.step(&[&commands[0],&commands[1]]).unwrap()) {support::fold(session,stream);}
    let mut lua=LuaTeam::new(sessions[index].team(),&[BotSeat{slot:SlotId(index as u8),hero:HeroId::SHADOW_FIEND}],Limits::DEFAULT,Keys{hash:[1,2],random:3}).unwrap();
    lua.add_script(SlotId(index as u8),ScriptKind::Takeover,"forest",br#"
      local enemy=nil
      function Think()
        local bot=GetBot(); local trees=bot:GetNearbyTrees(256); assert(type(trees)=='table')
        local found=false; for _,id in ipairs(trees) do if id==0 then found=true end end; assert(found)
        assert(not pcall(bot.GetNearbyTrees,{})); assert(not pcall(bot.GetNearbyTrees,bot,0/0))
        if enemy==nil then enemy=GetUnitList(UNIT_LIST_ENEMY_HEROES)[1]; assert(enemy and type(enemy:GetNearbyTrees(256))=='table'); assert(enemy:NumQueuedActions()==-1)
        else assert(enemy:GetNearbyTrees(256)==nil); assert(enemy:NumQueuedActions()==-1) end
        bot:Action_MoveToLocation(bot:GetLocation())
      end
    "#).unwrap();
    let turn=lua.think(&sessions[index]).unwrap();assert!(turn.failures.is_empty(),"{:?}",turn.failures);assert_eq!(turn.bots[0].orders.len(),1);
    let map=sessions[index].map();let far=map.fountains[1-index];
    let enemy=1-index;
    let command=sessions[enemy].issue(IssuedOrder::hero(Order::Cheat{cheat:Cheat::Teleport {at:far}})).unwrap();
    let mut commands=[Vec::new(),Vec::new()];commands[enemy].push(command);
    for (session,stream) in sessions.iter_mut().zip(local.step(&[&commands[0],&commands[1]]).unwrap()) {support::fold(session,stream);}
    assert!(!sessions[index].current().unwrap().units.iter().any(|unit|unit.kind==bota_proto::UnitKind::Hero && unit.team==sessions[index].team().enemy()),"fixture must lose sight: index={index} tree={tree:?} far={far:?} units={:?}",sessions[index].current().unwrap().units.iter().filter(|unit|unit.kind==bota_proto::UnitKind::Hero).map(|unit|(unit.team,unit.pos)).collect::<Vec<_>>());
    let turn=lua.think(&sessions[index]).unwrap();assert!(turn.failures.is_empty(),"{:?}",turn.failures);assert_eq!(turn.bots[0].orders.len(),1);
 }
}
