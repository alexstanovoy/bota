#![cfg(all(feature="native",not(target_arch="wasm32")))]
mod support;
use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat,Keys,LuaTeam,ScriptKind};
use bota_proto::{HeroId,Order,SlotId,Target,Team,Vec2};
fn host()->LuaTeam {
    let seats=[BotSeat {slot:SlotId(0),hero:HeroId::SHADOW_FIEND}];let keys=Keys {hash:[17,19],random:23};
    let limits=bota_lua::native::NativeConfig::new(Team::Radiant,seats.to_vec(),keys).limits;
    LuaTeam::new_measured(Team::Radiant,&seats,limits,keys,true).unwrap()
}
fn advance(local:&mut bota_server::local::LocalMatch,sessions:&mut[bota_client_core::ClientSession],orders:&[IssuedOrder]) {
    let commands:Vec<_>=orders.iter().map(|&order|sessions[0].issue(order).unwrap()).collect();
    for (session,stream) in sessions.iter_mut().zip(local.step(&[&commands,&[]]).unwrap()) {support::fold(session,stream);}
}
fn marker(x:i32,y:i32)->IssuedOrder {IssuedOrder::hero(Order::Move {target:Target::Pos(Vec2::from_ints(9216+x,9216+y))})}
// Independent fixtures preserve the packs' relevant callback composition: complete script
// takeover, an error caught inside Think, an uncaught auxiliary error after issuing an intent,
// and a later purchase failure. They contain no third-party script text.
#[test]
fn sustained_script_intents_survive_caught_and_uncaught_callback_errors() {
    let (mut local,mut sessions)=support::local_with_cheats(&[Team::Radiant,Team::Dire],bota_core::MAP2_ID);
    advance(&mut local,&mut sessions,&[IssuedOrder::hero(Order::Cheat {cheat:bota_proto::Cheat::StartGame})]);
    let mut lua=host();
    lua.add_script(SlotId(0),ScriptKind::TeamDesires,"team_fixture",b"function TeamThink() error('team fixture interruption') end").unwrap();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"takeover_fixture",b"local frame=0 function Think() frame=frame+1;pcall(function() GetBot():Action_MoveToLocation(Vector(-7000+frame*3,-6701,0));error('caught body interruption') end) end").unwrap();
    lua.add_script(SlotId(0),ScriptKind::AbilityItemUsage,"auxiliary_fixture",b"local frame=0 function AbilityUsageThink() frame=frame+1;GetBot():Action_MoveToLocation(Vector(-7000+frame*3,-6702,0));error('auxiliary fixture interruption') end function ItemUsageThink() GetBot():Action_MoveToLocation(Vector(-7000+frame*3,-6703,0)) end").unwrap();
    lua.add_script(SlotId(0),ScriptKind::ItemPurchase,"purchase_fixture",b"function ItemPurchaseThink() GetBot():ActionImmediate_PurchaseItem('item_branches');error('purchase fixture interruption') end").unwrap();
    let start=sessions[0].own_hero().unwrap().pos;
    for frame in 1..=300 {
        let turn=lua.think(&sessions[0]).unwrap();assert_eq!(turn.hook_activity.native,0,"native defaults cannot supply any accepted marker");
        assert_eq!(turn.failures.len(),3);for message in ["team fixture interruption","auxiliary fixture interruption","purchase fixture interruption"] {assert!(turn.failures.iter().any(|failure|failure.error.to_string().contains(message)),"{message}: {:?}",turn.failures);}
        let body:Vec<_>=turn.bots[0].orders.iter().copied().filter(|issued|matches!(issued.order,Order::Move {..}|Order::Attack {..}|Order::Cast {..}|Order::Use {..})).collect();
        assert_eq!(body,vec![marker(-7000+frame*3,-6701),marker(-7000+frame*3,-6702),marker(-7000+frame*3,-6703)],"script-only marker sequence at frame {frame}");
        advance(&mut local,&mut sessions,&turn.bots[0].orders);
    }
    assert_ne!(sessions[0].own_hero().unwrap().pos,start,"the real match applied the sustained movement orders");
    assert!(sessions[0].current().unwrap().game_tick()>0,"the stretch is actual game play");
}
#[test]
fn an_error_every_tick_keeps_only_that_ticks_script_order() {
    let (mut local,mut sessions)=support::local(&[Team::Radiant,Team::Dire]);let mut lua=host();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"repeated_failure",b"local frame=0 function Think() frame=frame+1;GetBot():Action_MoveToLocation(Vector(frame,73,0));error('repeated fixture failure') end").unwrap();
    for frame in 1..=12 {let turn=lua.think(&sessions[0]).unwrap();assert_eq!(turn.bots[0].orders,vec![marker(frame,73)]);assert_eq!(turn.failures.len(),1);assert!(turn.failures[0].error.to_string().contains("repeated fixture failure"));assert_eq!(turn.hook_activity.native,0);advance(&mut local,&mut sessions,&turn.bots[0].orders);}
}
#[test]
fn sixty_four_orders_before_an_error_leave_no_room_for_later_callbacks() {
    let (mut local,mut sessions)=support::local(&[Team::Radiant,Team::Dire]);let mut lua=host();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"full_error_outbox",b"local frame=0 function Think() frame=frame+1;for i=1,64 do GetBot():ActionImmediate_SwapItems(0,1) end error('full outbox fixture failure') end").unwrap();
    lua.add_script(SlotId(0),ScriptKind::AbilityItemUsage,"late_outbox",b"function AbilityUsageThink() GetBot():ActionImmediate_SwapItems(0,1) end function ItemUsageThink() GetBot():ActionImmediate_SwapItems(0,1) end").unwrap();
    for _ in 1..=4 {let turn=lua.think(&sessions[0]).unwrap();assert_eq!(turn.bots[0].orders,vec![IssuedOrder::hero(Order::Swap {from:bota_proto::ItemSlot(0),to:bota_proto::ItemSlot(1)});64]);assert_eq!(turn.failures.len(),3);assert!(turn.failures[0].error.to_string().contains("full outbox fixture failure"));assert_eq!(turn.failures.iter().filter(|failure|failure.error.to_string().contains("order limit exceeded")).count(),2);advance(&mut local,&mut sessions,&turn.bots[0].orders);}
}
#[test]
fn failed_initializers_retry_next_tick_and_suppress_their_hook_defaults() {
    let (mut local,mut sessions)=support::local(&[Team::Radiant,Team::Dire]);let mut lua=host();
    for mode in 1..=22 {let source=format!("function GetDesire() return {} end function Think() end",u8::from(mode==1));lua.add_script(SlotId(0),ScriptKind::Mode(mode),"mode_fixture",source.as_bytes()).unwrap();}
    lua.add_script(SlotId(0),ScriptKind::AbilityItemUsage,"usage_init_fixture",b"GetBot():Action_MoveToLocation(Vector(10+GameTime()*30,81,0));error('usage init fixture failure')").unwrap();
    lua.add_script(SlotId(0),ScriptKind::ItemPurchase,"purchase_init_fixture",b"GetBot():Action_MoveToLocation(Vector(20+GameTime()*30,82,0));error('purchase init fixture failure')").unwrap();
    for frame in 0..8 {let tick=sessions[0].current().unwrap().tick as i32;let turn=lua.think(&sessions[0]).unwrap();assert_eq!(turn.hook_activity.native,0,"failed source overrides suppress native defaults for their hooks");assert_eq!(turn.hook_activity.initializations,if frame==0 {24} else {2},"one attempt per source per tick, then retry next tick");assert_eq!(turn.bots[0].orders,vec![marker(10+tick,81),marker(20+tick,82)]);assert_eq!(turn.failures.len(),2);assert!(turn.failures.iter().any(|failure|failure.error.to_string().contains("usage init fixture failure")));assert!(turn.failures.iter().any(|failure|failure.error.to_string().contains("purchase init fixture failure")));advance(&mut local,&mut sessions,&turn.bots[0].orders);}
}
