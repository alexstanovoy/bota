mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};
fn check(session: &bota_client_core::ClientSession, index: usize, source: &[u8]) {
    let slot = SlotId(index as u8);
    let mut lua = LuaTeam::new(
        session.team(),
        &[BotSeat {
            slot,
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .unwrap();
    lua.add_script(slot, ScriptKind::Takeover, "shared_query", source)
        .unwrap();
    let turn = lua.think(session).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}
fn both(source: &[u8]) {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (index, session) in sessions.iter().enumerate() {
        check(session, index, source);
    }
}
#[test]
fn unspent_points_count_one_shared_raze_rank_group() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for index in 0..2 {
        check(
            &sessions[index],
            index,
            b"function Think() assert(GetBot():GetAbilityPoints()==1) end",
        );
        let issued = sessions[index]
            .issue(bota_client_core::IssuedOrder::hero(
                bota_proto::Order::Learn {
                    slot: bota_proto::AbilitySlot(0),
                },
            ))
            .unwrap();
        let mut commands = [Vec::new(), Vec::new()];
        commands[index].push(issued);
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&commands[0], &commands[1]]).unwrap())
        {
            support::fold(session, stream);
        }
        check(
            &sessions[index],
            index,
            b"function Think() assert(GetBot():GetAbilityPoints()==0) end",
        );
    }
}
#[test]
fn projectile_speed_comes_from_the_shared_body_definition() {
    both(b"function Think() assert(GetBot():GetAttackProjectileSpeed()==1200) end");
}
#[test]
fn base_damage_variance_is_half_the_base_damage_interval() {
    both(b"function Think() assert(GetBot():GetBaseDamageVariance()==3) end");
}
#[test]
fn stash_value_counts_a_nonempty_received_stash() {
    let (_, sessions) = support::courier_delivery_setup();
    check(&sessions[0],0,b"function Think() assert(GetBot():GetStashValue()==55,tostring(GetBot():GetStashValue())) end");
}
#[test]
fn extrapolation_without_a_second_observation_keeps_the_received_position() {
    both(b"function Think() local p=GetBot():GetLocation(); local q=GetBot():GetExtrapolatedLocation(2); assert(q.x==p.x and q.y==p.y) end");
}
#[test]
fn attack_target_is_nil_when_the_seat_observes_no_attack() {
    both(b"function Think() assert(GetBot():GetAttackTarget()==nil) end");
}
#[test]
fn bot_identity_uses_the_same_explicit_declaration_as_player_identity() {
    both(b"function Think() assert(GetBot():IsBot()); assert(IsPlayerBot(GetBot():GetPlayerID())) end");
}
#[test]
fn incoming_damage_uses_the_shared_damage_rules() {
    both(b"function Think() local bot=GetBot(); assert(bot:GetActualIncomingDamage(100,DAMAGE_TYPE_PURE)==100); assert(bot:GetActualIncomingDamage(100,DAMAGE_TYPE_MAGICAL)==73,tostring(bot:GetActualIncomingDamage(100,DAMAGE_TYPE_MAGICAL))); assert(bot:GetActualIncomingDamage(0,DAMAGE_TYPE_PHYSICAL)==0) end");
}
#[test]
fn nearby_lane_creeps_do_not_include_base_buildings_or_couriers() {
    both(b"function Think() assert(#GetBot():GetNearbyLaneCreeps(1600,false)==0); assert(#GetBot():GetNearbyLaneCreeps(1600,true)==0) end");
}
#[test]
fn nearby_neutrals_do_not_include_allied_units() {
    both(b"function Think() assert(#GetBot():GetNearbyNeutralCreeps(1600)==0) end");
}
#[test]
fn secret_shop_distance_uses_public_shop_geometry() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (index, session) in sessions.iter().enumerate() {
        let unit = session.own_hero().unwrap();
        let distance =
            bota_core::distance_from_secret_shop(session.map(), unit.pos, unit.bound).unwrap();
        assert!(distance > 0);
        check(
            session,
            index,
            format!("function Think() assert(GetBot():DistanceFromSecretShop()=={distance}) end")
                .as_bytes(),
        );
    }
}
#[test]
fn populated_nearby_queries_and_attack_target_follow_a_received_encounter() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, Order, Target, Vec2};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let at = Vec2::from_ints(9216, 9216);
    for index in 0..2 {
        let hero = local.hero_of(index).unwrap();
        local.world_mut().cheat(
            SlotId(index as u8),
            Some(hero),
            Cheat::Teleport {
                at: at + Vec2::from_ints(index as i32 * 400, 0),
            },
            &mut Vec::new(),
        );
    }
    for cheat in [
        Cheat::StartGame,
        Cheat::ClearModifiers {
            target: Target::None,
        },
        Cheat::CreateCreeps {
            enemy: true,
            at: at + Vec2::from_ints(100, 0),
        },
    ] {
        local
            .world_mut()
            .cheat(SlotId(0), None, cheat, &mut Vec::new());
    }
    local.world_mut().spawn_neutral(
        bota_core::NeutralKind::Kobold,
        at + Vec2::from_ints(200, 0),
        0,
        0,
    );
    for (session, stream) in sessions.iter_mut().zip(local.step(&[&[], &[]]).unwrap()) {
        support::fold(session, stream);
    }
    check(&sessions[0],0,b"function Think() assert(#GetBot():GetNearbyLaneCreeps(1600,true)>0,'lane'); assert(#GetBot():GetNearbyNeutralCreeps(1600)>0,'neutral') end");
    let target = sessions[0]
        .current()
        .unwrap()
        .units
        .iter()
        .find(|unit| unit.team == Team::Dire && bota_core::kind_def(unit.kind).lane_creep)
        .unwrap()
        .id;
    let attack = sessions[0]
        .issue(IssuedOrder::hero(Order::Attack {
            target: Target::Unit(target),
        }))
        .unwrap();
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[attack], &[]]).unwrap())
    {
        support::fold(session, stream);
    }
    check(&sessions[0],0,b"function Think() local target=GetBot():GetAttackTarget(); assert(target and target:GetTeam()==TEAM_DIRE and target:CanBeSeen(),'attack target') end");
}
#[test]
fn extrapolation_projects_the_received_nonzero_velocity() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Order, Target, Vec2};
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let at = sessions[0].own_hero().unwrap().pos;
    let order = sessions[0]
        .issue(IssuedOrder::hero(Order::Move {
            target: Target::Pos(at + Vec2::from_ints(1000, 0)),
        }))
        .unwrap();
    for tick in 0..60 {
        let commands = if tick == 0 { vec![order] } else { Vec::new() };
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&commands, &[]]).unwrap())
        {
            support::fold(session, stream);
        }
        let own = sessions[0].own_hero().unwrap();
        if sessions[0]
            .entity(own.id)
            .unwrap()
            .velocity
            .is_some_and(|velocity| velocity.delta != Vec2::ZERO)
        {
            check(&sessions[0],0,b"function Think() local bot=GetBot(); local p=bot:GetLocation(); local q=bot:GetExtrapolatedLocation(1); assert(q.x~=p.x or q.y~=p.y) end");
            return;
        }
    }
    panic!("the real movement order must yield observed displacement");
}
#[test]
fn hostile_received_armor_and_large_script_damage_are_rejected_before_core_arithmetic() {
    use bota_proto::{Fixed, ServerMsg};
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let own = sessions[0].own_hero().unwrap().id;
    let mut view = sessions[0].current().unwrap().clone();
    view.tick += 1;
    view.units
        .iter_mut()
        .find(|unit| unit.id == own)
        .unwrap()
        .armor = Fixed::MIN;
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .unwrap();
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .unwrap();
    check(&sessions[0],0,b"function Think() local ok,why=pcall(GetBot().GetActualIncomingDamage,GetBot(),2147483647,DAMAGE_TYPE_PHYSICAL); assert(not ok and string.find(tostring(why),'received damage statistics exceed projection range',1,true)) end");
}
