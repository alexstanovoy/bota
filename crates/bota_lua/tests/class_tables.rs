mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId,SlotId,Team};
fn run(source:&[u8]) {
    let (_,sessions)=support::local(&[Team::Radiant,Team::Dire]);
    for (index,session) in sessions.iter().enumerate() {
        let mut lua=LuaTeam::new(session.team(),&[BotSeat{slot:SlotId(index as u8),hero:HeroId::SHADOW_FIEND}],Limits::DEFAULT,Keys{hash:[1,2],random:3}).unwrap();
        lua.add_script(SlotId(index as u8),ScriptKind::Takeover,"classes",source).unwrap();
        let turn=lua.think(session).unwrap(); assert!(turn.failures.is_empty(),"{:?}",turn.failures);
    }
}
#[test] fn bot_class_overrides_are_looked_up_by_existing_handles() { run(br#"
function Think()
 local bot=GetBot(); local base=CDOTA_Bot_Script.GetTeam; local original=bot:GetTeam()
 assert(type(base)=='function')
 CDOTA_Bot_Script.GetTeam=function(self) return base(self)+100 end
 assert(bot:GetTeam()==original+100 and GetBot()==bot)
 CDOTA_Bot_Script.SharedMethod=function(self) return self:GetTeam() end
 assert(bot:SharedMethod()==original+100)
 bot.SharedMethod=function() return 7 end; assert(bot:SharedMethod()==7)
 assert(not pcall(base,{})); CDOTA_Bot_Script.GetTeam=base
end"#); }
#[test] fn ability_class_overrides_are_looked_up_by_existing_handles() { run(br#"
function Think()
 local a=GetBot():GetAbilityByName("nevermore_shadowraze1"); assert(a)
 local base=CDOTABaseAbility_BotScript.GetLevel; local level=a:GetLevel()
 CDOTABaseAbility_BotScript.GetLevel=function(self) return base(self)+10 end
 assert(a:GetLevel()==level+10)
 assert(not pcall(base,{})); CDOTABaseAbility_BotScript.GetLevel=base
end"#); }
