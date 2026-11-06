mod support;
use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team, Order, ItemSlot, ServerMsg, Kit};
fn team(slots: &[u8]) -> LuaTeam {
    LuaTeam::new(Team::Radiant, &slots.iter().map(|&slot| BotSeat { slot: SlotId(slot), hero: HeroId::SHADOW_FIEND }).collect::<Vec<_>>(), Limits::DEFAULT, Keys { hash:[17,19], random:23 }).expect("team")
}
#[test]
fn a_cached_item_cannot_activate_a_replacement_in_its_old_slot() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for name in ["item_clarity", "item_branches"] {
        let item = bota_core::item_by_key(name).expect("item");
        let order = sessions[0].issue(IssuedOrder::hero(Order::Buy { item })).expect("buy");
        let streams = local.step(&[&[order], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) { support::fold(session, stream); }
    }
    let mut lua = team(&[0]);
    lua.add_script(SlotId(0), ScriptKind::Takeover, "stale_item", b"local item function Think() if item == nil then item=GetBot():GetItemInSlot(0) assert(item~=nil) else GetBot():Action_UseAbility(item) end end").expect("script");
    assert!(lua.think(&sessions[0]).expect("cache").failures.is_empty());
    let swap = sessions[0].issue(IssuedOrder::hero(Order::Swap { from: ItemSlot(0), to: ItemSlot(1) })).expect("swap");
    let streams = local.step(&[&[swap], &[]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) { support::fold(session, stream); }
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.bots[0].orders.is_empty());
    assert!(turn.failures[0].error.to_string().contains("stale ability handle"));
}
#[test]
fn bot_state_getters_read_the_receiver_instead_of_the_executing_seat() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire, Team::Radiant]);
    let mut lua = team(&[0,2]);
    for (slot, mode, value, desire) in [(0,1,17,"0.25"),(2,2,29,"0.75")] {
        let source = format!(r#"local calls=0 function GetDesire() return {desire} end
        function OnStart() local b=GetBot() b:SetTarget(b) b:SetNextItemPurchaseValue({value}) end
        function Think() calls=calls+1 if calls>1 then local a,b=GetTeamMember(1),GetTeamMember(2)
        assert(a:GetTarget()==a and b:GetTarget()==b)
        assert(a:GetNextItemPurchaseValue()==17 and b:GetNextItemPurchaseValue()==29)
        assert(a:GetActiveMode()==1 and b:GetActiveMode()==2)
        assert(a:GetActiveModeDesire()==0.25 and b:GetActiveModeDesire()==0.75) end end"#);
        lua.add_script(SlotId(slot),ScriptKind::Mode(mode),"receiver",source.as_bytes()).expect("script");
    }
    assert!(lua.think(&sessions[0]).expect("first").failures.is_empty());
    let streams=local.step(&[&[],&[],&[]]).expect("step");
    for(session,stream)in sessions.iter_mut().zip(streams){support::fold(session,stream);}
    let turn=lua.think(&sessions[0]).expect("second");
    assert!(turn.failures.is_empty(),"{:?}",turn.failures);
}
#[test]
fn respawn_time_converts_six_hundred_protocol_ticks_to_twenty_seconds() {
    let (_,mut sessions)=support::local(&[Team::Radiant,Team::Dire]);
    let mut view=sessions[0].current().expect("view").clone();view.tick+=1;
    let hero=view.units.iter().find(|unit| unit.owner==Some(SlotId(0))&&unit.hero.is_some()).expect("hero").clone();
    view.units.retain(|unit| unit.id!=hero.id);
    let player=view.players.iter_mut().find(|player|player.slot==SlotId(0)).expect("player");
    player.respawn_left=600;player.kit=Some(Kit{abilities:hero.abilities,items:hero.items,town_portal:hero.town_portal});
    let tick=view.tick;
    sessions[0].ingest(ServerMsg::Snapshot{view},&mut()).expect("dead snapshot");
    sessions[0].ingest(ServerMsg::Events{tick,events:Vec::new()},&mut()).expect("events");
    let mut lua=team(&[0]);
    lua.add_script(SlotId(0),ScriptKind::Takeover,"respawn",b"function Think() assert(not GetBot():IsAlive()) assert(GetBot():GetRespawnTime()==20) end").expect("script");
    let turn=lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(),"{:?}",turn.failures);
}

#[test]
fn bot_state_survives_the_dead_heros_unit_track_expiring() {
    let (_,mut sessions)=support::local(&[Team::Radiant,Team::Dire]);
    let mut lua=team(&[0]);
    lua.add_script(SlotId(0),ScriptKind::Takeover,"remember",b"local saved function Think() local b=GetBot() if saved==nil then saved=b b:SetTarget(b) b:SetNextItemPurchaseValue(17) else assert(b==saved) assert(b:GetTarget()==b) assert(b:GetNextItemPurchaseValue()==17) assert(not b:IsNull()) end end").expect("script");
    assert!(lua.think(&sessions[0]).expect("first").failures.is_empty());
    let mut view=sessions[0].current().expect("view").clone();
    view.tick+=bota_client_core::SessionOptions::default().forget_after_ticks+1;
    let hero=view.units.iter().find(|unit|unit.owner==Some(SlotId(0))&&unit.hero.is_some()).expect("hero").clone();
    view.units.retain(|unit|unit.id!=hero.id);
    let player=view.players.iter_mut().find(|player|player.slot==SlotId(0)).expect("player");
    player.respawn_left=600; player.kit=Some(Kit{abilities:hero.abilities,items:hero.items,town_portal:hero.town_portal});
    let tick=view.tick;
    sessions[0].ingest(ServerMsg::Snapshot{view},&mut()).expect("snapshot");
    sessions[0].ingest(ServerMsg::Events{tick,events:Vec::new()},&mut()).expect("events");
    assert!(!sessions[0].entities().iter().any(|track| track.id==hero.id),"fixture expired the unit track");
    let turn=lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(),"{:?}",turn.failures);
}
