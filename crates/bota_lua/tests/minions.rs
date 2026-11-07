mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, ServerMsg, SlotId, Team};

fn team(slots: &[u8]) -> LuaTeam {
    let seats: Vec<_> = slots.iter().map(|&slot| BotSeat { slot: SlotId(slot), hero: HeroId::SHADOW_FIEND }).collect();
    LuaTeam::new(Team::Radiant, &seats, Limits::DEFAULT, Keys { hash: [1, 2], random: 3 }).expect("team")
}

#[test]
fn minion_think_moves_its_courier_through_the_real_order_path() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let courier = sessions[0].own_courier().expect("owned courier").clone();
    let mut lua = team(&[0]);
    lua.add_script(SlotId(0), ScriptKind::Minion, "minion", b"function MinionThink(unit) unit:Action_MoveToLocation(Vector(0,0,0)) end").expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
    let order = turn.bots[0].orders[0];
    assert_eq!(order.unit, Some(courier.id));
    let mut first = Some(sessions[0].issue(order).expect("courier order"));
    for _ in 0..30 {
        let streams = local.step(&[first.take().as_slice(), &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            assert!(!stream.iter().any(|message| matches!(message, ServerMsg::OrderRejected { .. })), "courier move is accepted");
            support::fold(session, stream);
        }
    }
    assert_ne!(sessions[0].own_courier().expect("courier still alive").pos, courier.pos);
}

#[test]
fn a_bot_cannot_move_another_seats_cached_courier() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Radiant, Team::Dire]);
    let mut lua = team(&[0, 1]);
    lua.add_script(SlotId(0), ScriptKind::Minion, "save_courier", b"function MinionThink(unit) _G.saved_courier=unit end").expect("script");
    lua.add_script(SlotId(1), ScriptKind::Takeover, "foreign_courier", br#"
        function Think()
            assert(_G.saved_courier)
            local ok,err=pcall(function() _G.saved_courier:Action_MoveToLocation(Vector(0,0,0)) end)
            assert(not ok and string.find(err,'visible living courier',1,true))
        end
    "#).expect("script");
    let turn=lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots.iter().all(|bot| bot.orders.is_empty()));
}

#[test]
fn a_cached_courier_absent_from_the_current_view_cannot_be_moved() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let courier = sessions[0].own_courier().expect("owned courier").id;
    let mut lua = team(&[0]);
    lua.add_script(SlotId(0), ScriptKind::Takeover, "stale_courier", br#"
        function Think()
            local saved=GetBot().saved_courier
            if saved then
                local ok,err=pcall(function() saved:Action_MoveToLocation(Vector(0,0,0)) end)
                assert(not ok and string.find(err,'visible living courier',1,true))
            end
        end
    "#).expect("script");
    lua.add_script(SlotId(0), ScriptKind::Minion, "save_courier", b"function MinionThink(unit) GetBot().saved_courier=unit end").expect("script");
    assert!(lua.think(&sessions[0]).expect("first think").failures.is_empty());
    let mut view=sessions[0].current().expect("view").clone();
    view.units.retain(|unit| unit.id != courier); view.tick += 1;
    let tick=view.tick;
    sessions[0].ingest(ServerMsg::Snapshot { view }, &mut ()).expect("view without courier");
    sessions[0].ingest(ServerMsg::Events { tick, events: Vec::new() }, &mut ()).expect("events");
    assert!(sessions[0].entity(courier).is_some(), "the remembered handle still exists");
    let turn=lua.think(&sessions[0]).expect("second think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.is_empty());
}
