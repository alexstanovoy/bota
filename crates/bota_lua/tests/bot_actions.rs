mod support;
use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, EventKind, Fixed, HeroId, Order, SlotId, Team, Vec2};
fn host(source: &[u8]) -> LuaTeam {
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .unwrap();
    lua.add_script(SlotId(0), ScriptKind::Takeover, "controller", source)
        .unwrap();
    lua
}
fn step(
    local: &mut bota_server::local::LocalMatch,
    sessions: &mut [bota_client_core::ClientSession],
    orders: &[IssuedOrder],
) -> Vec<Vec<bota_proto::ServerMsg>> {
    let outgoing: Vec<_> = orders
        .iter()
        .map(|&order| sessions[0].issue(order).unwrap())
        .collect();
    let streams = local.step(&[&outgoing, &[]]).unwrap();
    for (session, stream) in sessions.iter_mut().zip(&streams) {
        support::fold(session, stream.clone());
    }
    streams
}
#[test]
fn a_direct_move_is_the_controllers_current_action_while_the_unit_walks() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua=host(b"local issued=false function Think() local b=GetBot() if not issued then b:Action_MoveDirectly(b:GetLocation()+Vector(256,0,0));issued=true end assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_MOVE_TO_DIRECTLY) end");
    for _ in 0..3 {
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        step(&mut local, &mut sessions, &turn.bots[0].orders);
    }
}
#[test]
fn queued_attack_once_launches_one_attack_then_the_queued_move_reaches_its_target() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let at = Vec2::from_ints(9216, 9216);
    step(
        &mut local,
        &mut sessions,
        &[
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Teleport { at },
            }),
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::CreateCreeps {
                    enemy: true,
                    at: at + Vec2::from_ints(224, 0),
                },
            }),
        ],
    );
    let own = sessions[0].own_hero().unwrap().id;
    let destination = at + Vec2::from_ints(0, 512);
    let mut lua=host(b"local issued=false function Think() if not issued then local b=GetBot();local c=b:GetNearbyCreeps(800,true);assert(#c>0);b:ActionQueue_AttackUnit(c[1],true);b:ActionQueue_MoveToLocation(Vector(0,512,0));issued=true end end");
    let mut launches = 0;
    let mut arrived = false;
    let mut launch_tick = None;
    let mut damage_tick = None;
    let mut queued_move_tick = None;
    for _ in 0..180 {
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        if turn.bots[0].orders.iter().any(|issued| {
            issued.order
                == Order::Move {
                    target: bota_proto::Target::Pos(destination),
                }
        }) {
            queued_move_tick = Some(sessions[0].current().unwrap().tick);
        }
        let streams = step(&mut local, &mut sessions, &turn.bots[0].orders);
        for message in &streams[0] {
            if let bota_proto::ServerMsg::Events { tick, events } = message {
                let count=events.iter().filter(|event|matches!(event,EventKind::AttackLaunched {attacker,..} if *attacker==own)).count();
                launches += count;
                if count > 0 {
                    launch_tick.get_or_insert(*tick);
                }
                if events.iter().any(|event|matches!(event,EventKind::Damaged {source:Some(source),..} if *source==own)) {damage_tick.get_or_insert(*tick);}
            }
        }
        arrived |= sessions[0]
            .own_hero()
            .unwrap()
            .pos
            .within(destination, Fixed::from_int(16));
    }
    assert_eq!(launches, 1, "exactly one attack before the queued move");
    assert!(
        damage_tick > launch_tick,
        "the ranged projectile has a real flight before damage"
    );
    assert_eq!(
        queued_move_tick, damage_tick,
        "the queued move begins after observed damage, not launch"
    );
    assert!(
        arrived,
        "client action queue must advance from the shot to the move"
    );
}

#[test]
fn clearing_without_stop_keeps_the_unit_order_but_not_the_controller_queue() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua=host(br#"local frame=0 function Think() frame=frame+1;local b=GetBot();if frame==1 then assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_IDLE);b:Action_MoveDirectly(b:GetLocation()+Vector(256,0,0));assert(b:NumQueuedActions()==1);assert(b:GetQueuedActionType(0)==BOT_ACTION_TYPE_MOVE_TO_DIRECTLY) elseif frame==2 then assert(b:NumQueuedActions()==0);assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_MOVE_TO_DIRECTLY);b:Action_ClearActions(false);assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_IDLE) elseif frame==3 then b:Action_ClearActions(true) end end"#);
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    step(&mut local, &mut sessions, &first.bots[0].orders);
    let before = sessions[0].own_hero().unwrap().pos;
    let second = lua.think(&sessions[0]).unwrap();
    assert!(second.failures.is_empty(), "{:?}", second.failures);
    assert!(second.bots[0].orders.is_empty());
    step(&mut local, &mut sessions, &second.bots[0].orders);
    assert_ne!(
        sessions[0].own_hero().unwrap().pos,
        before,
        "false did not send Stop to the still moving unit"
    );
    let third = lua.think(&sessions[0]).unwrap();
    assert!(third.failures.is_empty(), "{:?}", third.failures);
    assert_eq!(
        third.bots[0].orders,
        vec![IssuedOrder::hero(Order::Move {
            target: bota_proto::Target::None
        })]
    );
}
#[test]
fn another_bodys_pending_action_reserves_its_outbox_slot_before_a_stop() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua=host(br#"function Think() local b=GetBot();for i=1,63 do b:ActionImmediate_SwapItems(0,1) end local courier=GetCourier(0);assert(courier);courier:Action_MoveDirectly(courier:GetLocation()+Vector(128,0,0));local ok,why=pcall(b.Action_ClearActions,b,true);assert(not ok and string.find(tostring(why),'order limit exceeded',1,true)) end"#);
    let turn = lua.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 64);
    assert_eq!(
        turn.bots[0]
            .orders
            .iter()
            .filter(|order| order.unit.is_some() && matches!(order.order, Order::Move { .. }))
            .count(),
        1
    );
    assert_eq!(
        turn.bots[0]
            .orders
            .iter()
            .filter(|order| matches!(order.order, Order::Swap { .. }))
            .count(),
        63
    );
}

#[test]
fn a_dead_target_releases_the_next_client_action() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let at = Vec2::from_ints(9216, 9216);
    step(
        &mut local,
        &mut sessions,
        &[
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Teleport { at },
            }),
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::CreateCreeps {
                    enemy: true,
                    at: at + Vec2::from_ints(224, 0),
                },
            }),
        ],
    );
    let mut lua=host(b"local started=false function Think() local b=GetBot();if not started then b:ActionQueue_AttackUnit(b:GetNearbyCreeps(800,true)[1],false);b:ActionQueue_MoveToLocation(Vector(0,512,0));started=true else assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_MOVE_TO);assert(b:NumQueuedActions()==0) end end");
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    step(&mut local, &mut sessions, &first.bots[0].orders);
    step(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::KillCreeps {
                which: bota_proto::CreepSide::Dire,
            },
        })],
    );
    let second = lua.think(&sessions[0]).unwrap();
    assert!(second.failures.is_empty(), "{:?}", second.failures);
    assert_eq!(
        second.bots[0].orders,
        vec![IssuedOrder::hero(Order::Move {
            target: bota_proto::Target::Pos(at + Vec2::from_ints(0, 512))
        })]
    );
}
#[test]
fn an_ability_action_remains_current_during_cast_and_finishes_when_it_ends() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    step(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::MaxAbilities,
        })],
    );
    let mut lua=host(b"local started=false function Think() local b=GetBot();local a=b:GetAbilityByName('nevermore_shadowraze1');if not started then b:Action_UseAbility(a);assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_USE_ABILITY);started=true elseif a:GetCooldownTimeRemaining()>0 then assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_IDLE) else assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_USE_ABILITY) end end");
    let mut casting = false;
    let mut cast = false;
    for _ in 0..60 {
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        step(&mut local, &mut sessions, &turn.bots[0].orders);
        casting |= sessions[0].own_hero().unwrap().cast.is_some();
        cast |= sessions[0]
            .recent_events()
            .iter()
            .any(|event| matches!(event.kind, EventKind::AbilityCast { .. }));
    }
    assert!(
        casting && cast,
        "the fixture must traverse a real cast and effect"
    );
}

#[test]
fn a_queued_allied_attack_releases_tower_aggro_through_the_real_order_path() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let tower_at = sessions[0]
        .map()
        .dire_towers
        .iter()
        .find(|&&(lane, tier, _)| lane == bota_core::rules::LANE_MID && tier == 1)
        .unwrap()
        .2;
    let at = tower_at - Vec2::from_ints(600, 0);
    let ally_at = tower_at - Vec2::from_ints(300, 0);
    let ours: Vec<_> = [
        Cheat::Teleport { at },
        Cheat::CreateCreeps {
            enemy: false,
            at: ally_at,
        },
    ]
    .into_iter()
    .map(|cheat| {
        sessions[0]
            .issue(IssuedOrder::hero(Order::Cheat { cheat }))
            .unwrap()
    })
    .collect();
    let theirs = sessions[1]
        .issue(IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Teleport {
                at: tower_at - Vec2::from_ints(450, 0),
            },
        }))
        .unwrap();
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&ours, &[theirs]]).unwrap())
    {
        support::fold(session, stream);
    }
    let hero = sessions[0].own_hero().unwrap().id;
    let enemy = sessions[1].own_hero().unwrap().id;
    let tower = sessions[0]
        .current()
        .unwrap()
        .units
        .iter()
        .find(|unit| {
            unit.team == Team::Dire
                && unit.kind == bota_proto::UnitKind::Tower
                && unit.pos == tower_at
        })
        .unwrap()
        .id;
    let ally = sessions[0]
        .current()
        .unwrap()
        .units
        .iter()
        .filter(|unit| unit.team == Team::Radiant && bota_core::kind_def(unit.kind).creep)
        .min_by_key(|unit| (unit.pos.distance_squared(at), unit.id))
        .unwrap()
        .id;
    let amount =
        u16::try_from(sessions[0].current().unwrap().unit(ally).unwrap().hp * 2 / 3).unwrap();
    step(
        &mut local,
        &mut sessions,
        &[
            IssuedOrder::hero(Order::Attack {
                target: bota_proto::Target::Unit(enemy),
            }),
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Damage {
                    target: bota_proto::Target::Unit(ally),
                    amount,
                    kind: bota_proto::DamageKind::Pure,
                },
            }),
        ],
    );
    let mark = sessions[0].current().unwrap().unit(ally).unwrap();
    assert!(
        mark.hp < mark.max_hp / 2,
        "a deniable ally: {} / {}",
        mark.hp,
        mark.max_hp
    );
    assert_eq!(
        sessions[0]
            .current()
            .unwrap()
            .aggro
            .iter()
            .find(|aggro| aggro.unit == tower)
            .and_then(|aggro| aggro.target),
        Some(hero),
        "the attack on an enemy hero must first draw the tower"
    );
    let mut lua=host(b"local issued=false function Think() if not issued then local b=GetBot();local target=nil;for _,c in ipairs(b:GetNearbyCreeps(800,false)) do if c:GetHealth()<c:GetMaxHealth()/2 then target=c;break end end assert(target,'a damaged allied creep is required');b:ActionQueue_AttackUnit(target,true);b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(0,256,0));issued=true end end");
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    step(&mut local, &mut sessions, &first.bots[0].orders);
    let after = sessions[0]
        .current()
        .unwrap()
        .aggro
        .iter()
        .find(|aggro| aggro.unit == tower)
        .and_then(|aggro| aggro.target);
    assert!(
        after.is_some() && after != Some(hero),
        "the allied attack intent must release the hero to another tower target: {after:?}"
    );
    let mut moved = false;
    for _ in 0..90 {
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        moved |= turn.bots[0].orders.iter().any(|issued| {
            matches!(
                issued.order,
                Order::Move {
                    target: bota_proto::Target::Pos(_)
                }
            )
        });
        step(&mut local, &mut sessions, &turn.bots[0].orders);
    }
    assert!(moved, "the observed deny hit must release the queued move");
}

#[test]
fn a_death_and_respawn_between_thinks_cannot_keep_the_old_controller_tail() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let mut lua=host(b"local issued=false function Think() local b=GetBot();if not issued then b:Action_MoveToLocation(Vector(0,0,0));b:ActionQueue_MoveToLocation(Vector(256,0,0));issued=true else assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_IDLE);assert(b:NumQueuedActions()==0) end end");
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    step(&mut local, &mut sessions, &first.bots[0].orders);
    let before = sessions[0].own_hero().unwrap().id;
    step(
        &mut local,
        &mut sessions,
        &[
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Suicide,
            }),
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Respawn,
            }),
        ],
    );
    assert_eq!(
        sessions[0].own_hero().unwrap().id,
        before,
        "the stream reuses this hero body across respawn"
    );
    let next = lua.think(&sessions[0]).unwrap();
    assert!(next.failures.is_empty(), "{:?}", next.failures);
    assert!(next.bots[0].orders.is_empty());
}
