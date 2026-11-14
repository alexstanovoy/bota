mod support;
use bota_lua::{BotSeat,Keys,Limits,LuaTeam,ScriptKind};
use bota_proto::{HeroId,SlotId,Team};
#[test] fn lane_front_empty_creeps_and_tower_fronts_are_viewer_relative_on_both_sides() {
 let (_,sessions)=support::local(&[Team::Radiant,Team::Dire]);
 for (index,session) in sessions.iter().enumerate() {
  let mut lua=LuaTeam::new(session.team(),&[BotSeat {slot:SlotId(index as u8),hero:HeroId::SHADOW_FIEND}],Limits::DEFAULT,Keys{hash:[1,2],random:3}).unwrap();
  lua.add_script(SlotId(index as u8),ScriptKind::Takeover,"lane_front",br#"function Think()
    assert(GetLaneFrontAmount(GetTeam(),LANE_MID,true)==0)
    assert(GetLaneFrontAmount(GetOpposingTeam(),LANE_MID,true)==1)
    local front=GetLaneFrontAmount(GetTeam(),LANE_MID,false)
    assert(front>0 and front<0.5)
    assert(not pcall(GetLaneFrontAmount,GetTeam(),99,true))
    assert(not pcall(GetLaneFrontAmount,GetTeam(),LANE_MID,{}))
  end"#).unwrap();
  let result=lua.think(session).unwrap();assert!(result.failures.is_empty(),"{:?}",result.failures);
 }
}
