mod support;
use bota_lua::{BotSeat,Keys,Limits,LuaTeam,ScriptKind};
use bota_client_core::IssuedOrder;
use bota_proto::{HeroId,Order,SlotId,Target,Team,Vec2};
fn reader(expected:usize)->LuaTeam {
 let mut lua=LuaTeam::new(Team::Radiant,&[BotSeat{slot:SlotId(0),hero:HeroId::SHADOW_FIEND}],Limits::DEFAULT,Keys{hash:[1,2],random:3}).unwrap();
 lua.add_script(SlotId(0),ScriptKind::Takeover,"queue_count",format!("function Think() assert(GetBot():NumQueuedActions()=={expected}); assert(not pcall(GetBot().NumQueuedActions,{{}})); end").as_bytes()).unwrap();lua
}
#[test] fn queued_action_count_excludes_a_running_move_and_counts_only_waiting_orders() {
 let (mut local,mut sessions)=support::local(&[Team::Radiant,Team::Dire]);
 assert!(reader(0).think(&sessions[0]).unwrap().failures.is_empty());
 let position=sessions[0].own_hero().unwrap().pos;
 let first=sessions[0].issue(IssuedOrder::hero(Order::Move{target:Target::Pos(position+Vec2::from_ints(1000,0))})).unwrap();
 let mut intent=IssuedOrder::hero(Order::Move{target:Target::Pos(position+Vec2::from_ints(0,1000))});intent.queue=true;
 let second=sessions[0].issue(intent).unwrap();
 for (session,stream) in sessions.iter_mut().zip(local.step(&[&[first,second],&[]]).unwrap()) {support::fold(session,stream);}
 assert_eq!(sessions[0].own_hero().unwrap().queue.len(),1);
 let turn=reader(1).think(&sessions[0]).unwrap();assert!(turn.failures.is_empty(),"{:?}",turn.failures);
 let stop=sessions[0].issue(IssuedOrder::hero(Order::Move{target:Target::None})).unwrap();
 for (session,stream) in sessions.iter_mut().zip(local.step(&[&[stop],&[]]).unwrap()) {support::fold(session,stream);}
 assert_eq!(sessions[0].own_hero().unwrap().queue.len(),0);
 assert!(reader(0).think(&sessions[0]).unwrap().failures.is_empty());
}
