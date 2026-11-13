mod support;

use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, SlotId, Target, Team, Vec2};

fn assert_query_move(turn: &bota_lua::TeamTurn, attack_speed: i32) {
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
    assert_eq!(turn.bots[0].orders[0].unit, None);
    assert_eq!(turn.bots[0].orders[0].order, Order::Move {
        target: Target::Pos(Vec2::from_ints(9716, 9216 + attack_speed * 10)),
    });
}

fn fold_step(local: &mut bota_server::local::LocalMatch, sessions: &mut [ClientSession], by_seat: &[&[bota_proto::OutgoingOrder]]) {
    for (session, stream) in sessions.iter_mut().zip(local.step(by_seat).expect("step")) {
        assert!(!stream.iter().any(|message| matches!(message,bota_proto::ServerMsg::OrderRejected{..})), "the query move and purchase must be accepted");
        support::fold(session, stream);
    }
}

#[test]
fn attack_timing_uses_base_point_seconds_and_a_speed_multiplier_after_a_real_purchase() {
    let gloves = bota_core::item_by_key("item_gloves").expect("attack-speed item");
    for (index, side) in [(0,Team::Radiant),(1,Team::Dire)] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant,Team::Dire]);
        let mut lua = LuaTeam::new(side,&[BotSeat {slot:SlotId(index as u8),hero:HeroId::SHADOW_FIEND}],Limits::DEFAULT,Keys {hash:[17,19],random:23}).expect("team");
        lua.add_script(SlotId(index as u8),ScriptKind::Takeover,"=attack_timing_units",br#"
            function Think()
                local bot=GetBot()
                assert(math.abs(bot:GetAttackPoint()-0.5)<0.000001, 'base attack point must stay in seconds')
                assert(math.abs(bot:GetSecondsPerAttack()*bot:GetAttackSpeed()-1.6)<0.002, 'speed is normalized to one, not one hundred')
                bot:Action_MoveToLocation(Vector(math.floor(bot:GetAttackPoint()*1000+0.5),math.floor(bot:GetAttackSpeed()*1000+0.5),0))
            end
        "#).expect("script");
        let before = sessions[index].own_hero().expect("hero").clone();
        assert_eq!(before.attack_speed,125,"SF's base100 plus25agility");
        let first = lua.think(&sessions[index]).expect("before purchase");
        assert_query_move(&first,before.attack_speed);
        let mut orders:Vec<_> = first.bots[0].orders.iter().map(|&order|sessions[index].issue(order).expect("query move")).collect();
        orders.push(sessions[index].issue(IssuedOrder::hero(Order::Buy {item:gloves})).expect("purchase"));
        let mut by_seat=[Vec::new(),Vec::new()];
        by_seat[index]=orders;
        fold_step(&mut local,&mut sessions,&[&by_seat[0],&by_seat[1]]);
        let after=sessions[index].own_hero().expect("hero after purchase");
        assert!(after.items.iter().flatten().any(|item|item.id==gloves));
        assert!(after.attack_speed>before.attack_speed);
        assert!(after.attack_point<before.attack_point,"the wire's effective windup changes with attack speed");
        assert!(after.attack_time<before.attack_time);
        let speed=after.attack_speed;
        let next=lua.think(&sessions[index]).expect("after purchase");
        assert_query_move(&next,speed);
        let outgoing:Vec<_>=next.bots[0].orders.iter().map(|&order|sessions[index].issue(order).expect("second query move")).collect();
        let mut by_seat=[Vec::new(),Vec::new()];by_seat[index]=outgoing;
        fold_step(&mut local,&mut sessions,&[&by_seat[0],&by_seat[1]]);
    }
}
