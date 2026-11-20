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

#[test]
fn the_thirty_third_waiting_entry_is_rejected_without_changing_the_first_thirty_two() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua=host(br#"local frame=0 function Think() frame=frame+1;local b=GetBot();if frame==1 then for i=1,32 do b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256+i,0,0)) end local ok,why=pcall(b.ActionQueue_MoveToLocation,b,Vector(0,0,0));assert(not ok and string.find(tostring(why),'bot action queue limit exceeded',1,true));assert(b:NumQueuedActions()==32);assert(b:GetQueuedActionType(31)==BOT_ACTION_TYPE_MOVE_TO);assert(b:GetQueuedActionType(32)==-1) else assert(b:NumQueuedActions()==31) end end"#);
    for _ in 0..2 {
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        step(&mut local, &mut sessions, &turn.bots[0].orders);
    }
}
#[test]
fn a_couriers_waiting_move_is_independent_of_the_heros_queue() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let courier = sessions[0]
        .current()
        .unwrap()
        .units
        .iter()
        .find(|unit| unit.kind == bota_proto::UnitKind::Courier && unit.owner == Some(SlotId(0)))
        .unwrap()
        .id;
    let mut lua=host(br#"local frame=0 function Think() frame=frame+1;local b=GetBot();local c=GetCourier(0);if frame==1 then c:Action_MoveDirectly(c:GetLocation()+Vector(512,0,0));c:ActionQueue_MoveToLocation(c:GetLocation()+Vector(512,256,0));b:Action_MoveDirectly(b:GetLocation()+Vector(256,0,0));assert(c:NumQueuedActions()==2);assert(b:NumQueuedActions()==1) else assert(c:NumQueuedActions()==1);assert(b:NumQueuedActions()==0);assert(c:GetCurrentActionType()==BOT_ACTION_TYPE_MOVE_TO_DIRECTLY) end end"#);
    for frame in 0..2 {
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        if frame == 0 {
            assert_eq!(turn.bots[0].orders.len(), 2);
            assert!(
                turn.bots[0]
                    .orders
                    .iter()
                    .any(|order| order.unit == Some(courier))
            );
            assert!(turn.bots[0].orders.iter().any(|order| order.unit.is_none()));
        }
        step(&mut local, &mut sessions, &turn.bots[0].orders);
    }
}
#[test]
fn a_hero_that_stays_dead_has_no_current_or_waiting_action_on_later_ticks() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let mut lua=host(br#"local frame=0 function Think() frame=frame+1;local b=GetBot();if frame==1 then b:Action_MoveDirectly(b:GetLocation()+Vector(512,0,0));b:ActionQueue_MoveToLocation(Vector(0,256,0)) else assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_IDLE);assert(b:NumQueuedActions()==0) end end"#);
    let turn = lua.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty());
    step(&mut local, &mut sessions, &turn.bots[0].orders);
    step(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Suicide,
        })],
    );
    for _ in 0..5 {
        assert!(sessions[0].current().unwrap().players[0].respawn_left > 0);
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert!(turn.bots[0].orders.is_empty());
        step(&mut local, &mut sessions, &[]);
    }
}
#[test]
fn arriving_at_an_attack_move_releases_the_waiting_move() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let goal = sessions[0].own_hero().unwrap().pos + Vec2::from_ints(256, 0);
    let mut lua=host(br#"local frame=0 function Think() frame=frame+1;local b=GetBot();if frame==1 then b:Action_AttackMove(b:GetLocation());b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256,0,0)) else assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_MOVE_TO);assert(b:NumQueuedActions()==0) end end"#);
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty());
    step(&mut local, &mut sessions, &first.bots[0].orders);
    let next = lua.think(&sessions[0]).unwrap();
    assert!(next.failures.is_empty(), "{:?}", next.failures);
    assert_eq!(
        next.bots[0].orders,
        vec![IssuedOrder::hero(Order::Move {
            target: bota_proto::Target::Pos(goal)
        })]
    );
}
#[test]
fn an_unidentified_effect_releases_the_waiting_move_at_its_five_second_bound() {
    use bota_client_core::bot_actions::{BotAction, BotActionMode, BotActionType, BotActions};
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let hero = sessions[0].own_hero().unwrap();
    let goal = hero.pos + Vec2::from_ints(128, 0);
    let mut controller = BotActions::default();
    controller.bind(Some(hero.id));
    controller
        .request(
            BotAction {
                kind: BotActionType::UseAbility,
                order: Order::Cast {
                    slot: bota_proto::AbilitySlot(255),
                    target: bota_proto::Target::None,
                },
                effect: None,
                once: false,
            },
            BotActionMode::Queue,
        )
        .unwrap();
    controller
        .request(
            BotAction {
                kind: BotActionType::MoveTo,
                order: Order::Move {
                    target: bota_proto::Target::Pos(goal),
                },
                effect: None,
                once: false,
            },
            BotActionMode::Queue,
        )
        .unwrap();
    let started = sessions[0].current().unwrap().tick;
    let first = controller.start_ready(&sessions[0]).unwrap();
    step(&mut local, &mut sessions, &[IssuedOrder::hero(first)]);
    let bound = 5 * bota_core::rules::TICKS_PER_SECOND;
    for elapsed in 1..=bound {
        assert_eq!(sessions[0].current().unwrap().tick - started, elapsed);
        let order = controller.update(&sessions[0]);
        if elapsed < bound {
            assert!(order.is_none());
            step(&mut local, &mut sessions, &[]);
        } else {
            assert_eq!(
                order,
                Some(Order::Move {
                    target: bota_proto::Target::Pos(goal)
                })
            );
            step(
                &mut local,
                &mut sessions,
                &[IssuedOrder::hero(order.unwrap())],
            );
        }
    }
}

#[test]
fn an_instant_item_application_releases_its_tail_only_for_its_own_caster() {
    use bota_proto::{ItemId, ItemSlot, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let at = sessions[0].own_hero().unwrap().pos;
    let setup: Vec<Vec<_>> = (0..2)
        .map(|index| {
            let mut cheats = vec![
                Cheat::Item {
                    item: ItemId::MANGO,
                },
                Cheat::SetPools {
                    health: 100,
                    mana: 10,
                },
            ];
            if index == 1 {
                cheats.push(Cheat::Teleport {
                    at: at + Vec2::from_ints(224, 0),
                });
            }
            cheats
                .into_iter()
                .map(|cheat| {
                    sessions[index]
                        .issue(IssuedOrder::hero(Order::Cheat { cheat }))
                        .unwrap()
                })
                .collect()
        })
        .collect();
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&setup[0], &setup[1]]).unwrap())
    {
        support::fold(session, stream);
    }
    let own = sessions[0].own_hero().unwrap().id;
    let enemy = sessions[1].own_hero().unwrap().id;
    assert!(sessions[0].current().unwrap().unit(enemy).is_some());
    let mut lua=host(br#"local frame=0 function Think() frame=frame+1;local b=GetBot();if frame==1 then b:Action_UseAbility(b:GetItemInSlot(0));b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256,0,0)) elseif frame==2 then assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_USE_ABILITY,'another caster must not finish our action');assert(b:NumQueuedActions()==1) else assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_MOVE_TO,'our instant item effect must release the queued move');assert(b:NumQueuedActions()==0) end end"#);
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    assert_eq!(first.bots[0].orders.len(), 1);
    let enemy_use = sessions[1]
        .issue(IssuedOrder::hero(Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        }))
        .unwrap();
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[], &[enemy_use]]).unwrap())
    {
        support::fold(session, stream);
    }
    assert!(sessions[0].recent_events().iter().any(|event|matches!(event.kind,EventKind::ItemUsed {caster:Some(id),item:ItemId::MANGO,..} if id==enemy)));
    let other = lua.think(&sessions[0]).unwrap();
    assert!(other.failures.is_empty(), "{:?}", other.failures);
    assert!(other.bots[0].orders.is_empty());
    step(&mut local, &mut sessions, &first.bots[0].orders);
    assert!(sessions[0].recent_events().iter().any(|event|matches!(event.kind,EventKind::ItemUsed {caster:Some(id),item:ItemId::MANGO,..} if id==own)));
    assert!(
        !sessions[0]
            .recent_events()
            .iter()
            .any(|event| matches!(event.kind,EventKind::Damaged {source:Some(id),..} if id==own)),
        "the item's completion is not damage evidence"
    );
    let next = lua.think(&sessions[0]).unwrap();
    assert!(next.failures.is_empty(), "{:?}", next.failures);
    assert_eq!(
        next.bots[0].orders,
        vec![IssuedOrder::hero(Order::Move {
            target: Target::Pos(at + Vec2::from_ints(256, 0))
        })]
    );
    let before = sessions[0].own_hero().unwrap().pos;
    step(&mut local, &mut sessions, &next.bots[0].orders);
    assert_ne!(sessions[0].own_hero().unwrap().pos, before);
}

fn rejected_item_case(case: &str, expected: bota_proto::RejectReason) {
    use bota_proto::{ItemId, ItemSlot, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let item = match case {
        "mana" => bota_core::item_by_key("item_shivas_guard").unwrap(),
        "cooldown" => bota_core::item_by_key("item_phase_boots").unwrap(),
        "charges" => bota_core::item_by_key("item_magic_stick").unwrap(),
        "target_dies" | "wrong_target" => ItemId::HEALING_SALVE,
        _ => ItemId::MANGO,
    };
    let mut setup = vec![
        IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Item { item },
        }),
        IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::SetPools {
                health: 50,
                mana: 0,
            },
        }),
    ];
    if case == "swapped" {
        setup.push(IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Item {
                item: ItemId::IRON_BRANCH,
            },
        }));
    }
    if case == "target_dies" {
        setup.push(IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::CreateCreeps {
                enemy: false,
                at: sessions[0].own_hero().unwrap().pos + Vec2::from_ints(128, 0),
            },
        }));
    }
    if case == "not_ready" {
        setup.push(IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Teleport {
                at: Vec2::from_ints(9216, 9216),
            },
        }));
    }
    step(&mut local, &mut sessions, &setup);
    if case == "cooldown" {
        step(
            &mut local,
            &mut sessions,
            &[IssuedOrder::hero(Order::Use {
                slot: ItemSlot(0),
                target: Target::None,
            })],
        );
        assert!(
            sessions[0].own_hero().unwrap().items[0]
                .unwrap()
                .cooldown_left
                > 0
        );
    }
    if case == "not_ready" {
        step(
            &mut local,
            &mut sessions,
            &[IssuedOrder::hero(Order::Swap {
                from: ItemSlot(0),
                to: ItemSlot(6),
            })],
        );
        for _ in 0..=bota_core::rules::BACKPACK_GRACE_TICKS {
            step(&mut local, &mut sessions, &[]);
        }
        step(
            &mut local,
            &mut sessions,
            &[
                IssuedOrder::hero(Order::Swap {
                    from: ItemSlot(6),
                    to: ItemSlot(0),
                }),
                IssuedOrder::hero(Order::Cheat {
                    cheat: Cheat::SetPools {
                        health: 50,
                        mana: 0,
                    },
                }),
            ],
        );
        assert!(sessions[0].own_hero().unwrap().items[0].unwrap().mute_left > 0);
    }
    let use_call = match case {
        "target_dies" => {
            "local c=b:GetNearbyCreeps(800,false);assert(#c>0);b:Action_UseAbilityOnEntity(item,c[1])"
        }
        _ => "b:Action_UseAbility(item)",
    };
    let source = format!(
        "local started=false function Think() local b=GetBot();if not started then local item=b:GetItemInSlot(0);assert(item);{use_call};b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256,0,0));started=true end end"
    );
    let goal = sessions[0].own_hero().unwrap().pos + Vec2::from_ints(256, 0);
    let mut lua = host(source.as_bytes());
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty(), "{case}: {:?}", first.failures);
    assert_eq!(first.bots[0].orders.len(), 1);
    assert!(matches!(first.bots[0].orders[0].order, Order::Use { .. }));
    let mut before = Vec::new();
    match case {
        "empty" => before.push(Order::Swap {
            from: ItemSlot(0),
            to: ItemSlot(6),
        }),
        "sold" => before.push(Order::Sell { slot: ItemSlot(0) }),
        "swapped" => before.push(Order::Swap {
            from: ItemSlot(0),
            to: ItemSlot(1),
        }),

        "stunned" => before.push(Order::Cheat {
            cheat: Cheat::Stun {
                target: Target::None,
                ticks: 60,
            },
        }),
        "target_dies" => before.push(Order::Cheat {
            cheat: Cheat::KillCreeps {
                which: bota_proto::CreepSide::All,
            },
        }),
        "muted" => {
            let world = local.world_mut();
            let own = world.seats[0].unit.unwrap();
            world
                .modifiers
                .get_mut(own)
                .unwrap()
                .put(bota_server::game::Modifier::new(
                    bota_core::ModifierKind::HookFollowthrough,
                    None,
                    None,
                    Some(60),
                ));
            assert!(world.muted(own));
        }
        _ => {}
    }
    let mut orders: Vec<_> = before.into_iter().map(IssuedOrder::hero).collect();
    orders.extend_from_slice(&first.bots[0].orders);
    let streams = step(&mut local, &mut sessions, &orders);
    let refusals: Vec<_> = streams[0]
        .iter()
        .filter_map(|msg| {
            if let bota_proto::ServerMsg::OrderRejected { reason, .. } = msg {
                Some(*reason)
            } else {
                None
            }
        })
        .collect();
    assert!(refusals.contains(&expected), "{case}: {refusals:?}");
    assert!(!streams[0].iter().any(|msg|matches!(msg,bota_proto::ServerMsg::Events {events,..} if events.iter().any(|event|matches!(event,EventKind::ItemUsed {..})))),"{case}: refused uses must not claim application");
    let next = lua.think(&sessions[0]).unwrap();
    assert!(next.failures.is_empty(), "{case}: {:?}", next.failures);
    assert_eq!(
        next.bots[0].orders,
        vec![IssuedOrder::hero(Order::Move {
            target: Target::Pos(goal)
        })],
        "{case}: a refused item must release its tail"
    );
}
#[test]
fn item_controller_cooldown_refusal_releases_the_tail() {
    rejected_item_case("cooldown", bota_proto::RejectReason::OnCooldown);
}
#[test]
fn item_controller_muted_refusal_releases_the_tail() {
    rejected_item_case("muted", bota_proto::RejectReason::Disabled);
}
#[test]
fn item_controller_mana_refusal_releases_the_tail() {
    rejected_item_case("mana", bota_proto::RejectReason::NotEnoughMana);
}
#[test]
fn item_controller_empty_slot_refusal_releases_the_tail() {
    rejected_item_case("empty", bota_proto::RejectReason::EmptySlot);
}
#[test]
fn item_controller_sold_item_refusal_releases_the_tail() {
    rejected_item_case("sold", bota_proto::RejectReason::EmptySlot);
}
#[test]
fn item_controller_swapped_item_refusal_releases_the_tail() {
    rejected_item_case("swapped", bota_proto::RejectReason::WrongTargetKind);
}
#[test]
fn item_controller_stunned_refusal_releases_the_tail() {
    rejected_item_case("stunned", bota_proto::RejectReason::Disabled);
}
#[test]
fn item_controller_dead_target_refusal_releases_the_tail() {
    rejected_item_case("target_dies", bota_proto::RejectReason::UnknownTarget);
}
#[test]
fn item_controller_empty_charges_refusal_releases_the_tail() {
    rejected_item_case("charges", bota_proto::RejectReason::NoCharges);
}
#[test]
fn item_controller_backpack_wait_refusal_releases_the_tail() {
    rejected_item_case("not_ready", bota_proto::RejectReason::NotReady);
}
#[test]
fn item_controller_wrong_target_refusal_releases_the_tail() {
    rejected_item_case("wrong_target", bota_proto::RejectReason::WrongTargetKind);
}

#[test]
fn item_controller_success_survives_recent_event_eviction() {
    use bota_proto::{ItemId, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    step(
        &mut local,
        &mut sessions,
        &[
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Item {
                    item: ItemId::MANGO,
                },
            }),
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::SetPools {
                    health: 50,
                    mana: 0,
                },
            }),
        ],
    );
    let goal = sessions[0].own_hero().unwrap().pos + Vec2::from_ints(256, 0);
    let mut lua=host(b"local done=false function Think() if not done then local b=GetBot();b:Action_UseAbility(b:GetItemInSlot(0));b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256,0,0));done=true end end");
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty());
    step(&mut local, &mut sessions, &first.bots[0].orders);
    assert!(sessions[0].recent_events().iter().any(|event| matches!(
        event.kind,
        EventKind::ItemUsed {
            item: ItemId::MANGO,
            ..
        }
    )));
    let mut view = sessions[0].current().unwrap().clone();
    view.tick += 1;
    view.skipped_ticks.clear();
    let tick = view.tick;
    sessions[0]
        .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
        .unwrap();
    sessions[0]
        .ingest(
            bota_proto::ServerMsg::Events {
                tick,
                events: vec![
                    EventKind::TreeGrew { at: goal };
                    bota_client_core::MAX_RECENT_EVENTS + 1
                ],
            },
            &mut (),
        )
        .unwrap();
    assert!(
        !sessions[0]
            .recent_events()
            .iter()
            .any(|event| matches!(event.kind, EventKind::ItemUsed { .. }))
    );
    let next = lua.think(&sessions[0]).unwrap();
    assert!(next.failures.is_empty());
    assert_eq!(
        next.bots[0].orders,
        vec![IssuedOrder::hero(Order::Move {
            target: Target::Pos(goal)
        })],
        "an applied item must remain complete after unrelated events evict its report"
    );
}

#[test]
fn item_controller_an_unanswered_identified_use_expires_at_five_seconds() {
    use bota_proto::{ItemId, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    step(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Item {
                item: ItemId::MANGO,
            },
        })],
    );
    let goal = sessions[0].own_hero().unwrap().pos + Vec2::from_ints(256, 0);
    let mut lua=host(b"local done=false function Think() if not done then local b=GetBot();b:Action_UseAbility(b:GetItemInSlot(0));b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256,0,0));done=true end end");
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty());
    assert_eq!(first.bots[0].orders.len(), 1);
    for elapsed in 1..=5 * bota_core::rules::TICKS_PER_SECOND {
        step(&mut local, &mut sessions, &[]);
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty());
        if elapsed < 5 * bota_core::rules::TICKS_PER_SECOND {
            assert!(turn.bots[0].orders.is_empty(), "not before bound");
        } else {
            assert_eq!(
                turn.bots[0].orders,
                vec![IssuedOrder::hero(Order::Move {
                    target: Target::Pos(goal)
                })],
                "missing outcomes must not block the queue forever"
            );
        }
    }
}

#[test]
fn item_controller_two_queued_uses_require_two_applications() {
    use bota_proto::{ItemId, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    step(
        &mut local,
        &mut sessions,
        &[
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Item {
                    item: ItemId::MANGO,
                },
            }),
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Item {
                    item: ItemId::MANGO,
                },
            }),
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::SetPools {
                    health: 50,
                    mana: 0,
                },
            }),
        ],
    );
    assert!(
        sessions[0].own_hero().unwrap().items[0]
            .unwrap()
            .charges
            .unwrap()
            >= 2
    );
    let goal = sessions[0].own_hero().unwrap().pos + Vec2::from_ints(256, 0);
    use bota_client_core::bot_actions::{BotAction, BotActionMode, BotActions};
    let own = sessions[0].own_hero().unwrap();
    let mut controller = BotActions::default();
    controller.bind(Some(own.id));
    let use_item = BotAction::ordinary(
        Order::Use {
            slot: bota_proto::ItemSlot(0),
            target: Target::None,
        },
        own,
    )
    .unwrap();
    controller.request(use_item, BotActionMode::Queue).unwrap();
    controller.request(use_item, BotActionMode::Queue).unwrap();
    controller
        .request(
            BotAction::ordinary(
                Order::Move {
                    target: Target::Pos(goal),
                },
                own,
            )
            .unwrap(),
            BotActionMode::Queue,
        )
        .unwrap();
    for frame in 0..3 {
        let order = controller
            .update(&sessions[0])
            .expect("one new controller head");
        if frame < 2 {
            assert!(matches!(order, Order::Use { .. }));
        } else {
            assert_eq!(
                order,
                Order::Move {
                    target: Target::Pos(goal)
                }
            );
        }
        let streams = step(&mut local, &mut sessions, &[IssuedOrder::hero(order)]);
        let count = streams[0]
            .iter()
            .filter_map(|msg| {
                if let bota_proto::ServerMsg::Events { events, .. } = msg {
                    Some(events)
                } else {
                    None
                }
            })
            .flatten()
            .filter(|event| {
                matches!(
                    event,
                    EventKind::ItemUsed {
                        item: ItemId::MANGO,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(count, usize::from(frame < 2));
    }
}

#[test]
fn item_controller_observed_stun_sale_and_swap_fail_even_without_a_refusal_reply() {
    use bota_proto::{ItemId, ItemSlot, Target};
    for case in ["stun", "sale", "swap"] {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        step(
            &mut local,
            &mut sessions,
            &[
                IssuedOrder::hero(Order::Cheat {
                    cheat: Cheat::Item {
                        item: ItemId::MANGO,
                    },
                }),
                IssuedOrder::hero(Order::Cheat {
                    cheat: Cheat::Item {
                        item: ItemId::IRON_BRANCH,
                    },
                }),
            ],
        );
        let goal = sessions[0].own_hero().unwrap().pos + Vec2::from_ints(256, 0);
        let mut lua=host(b"local done=false function Think() if not done then local b=GetBot();b:Action_UseAbility(b:GetItemInSlot(0));b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256,0,0));done=true end end");
        let pending = lua.think(&sessions[0]).unwrap();
        assert!(pending.failures.is_empty());
        assert!(matches!(pending.bots[0].orders[0].order, Order::Use { .. }));
        // The item request is deliberately held while another ordinary operation changes its prerequisites.
        let mutation = match case {
            "stun" => Order::Cheat {
                cheat: Cheat::Stun {
                    target: Target::None,
                    ticks: 60,
                },
            },
            "sale" => Order::Sell { slot: ItemSlot(0) },
            _ => Order::Swap {
                from: ItemSlot(0),
                to: ItemSlot(1),
            },
        };
        let streams = step(&mut local, &mut sessions, &[IssuedOrder::hero(mutation)]);
        assert!(
            !streams[0]
                .iter()
                .any(|msg| matches!(msg, bota_proto::ServerMsg::OrderRejected { .. })),
            "{case}: the intervening operation succeeds"
        );
        let next = lua.think(&sessions[0]).unwrap();
        assert!(next.failures.is_empty());
        assert_eq!(
            next.bots[0].orders,
            vec![IssuedOrder::hero(Order::Move {
                target: Target::Pos(goal)
            })],
            "{case}: observation alone ends invalid work"
        );
    }
}

#[test]
fn item_controller_an_unanswered_use_without_a_tail_expires_and_sends_stop() {
    use bota_proto::{ItemId, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    step(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Item {
                item: ItemId::MANGO,
            },
        })],
    );
    let mut lua=host(b"local done=false function Think() if not done then local b=GetBot();b:Action_UseAbility(b:GetItemInSlot(0));done=true end end");
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty());
    assert_eq!(first.bots[0].orders.len(), 1);
    for elapsed in 1..=5 * bota_core::rules::TICKS_PER_SECOND {
        step(&mut local, &mut sessions, &[]);
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty());
        let expected = if elapsed == 5 * bota_core::rules::TICKS_PER_SECOND {
            vec![IssuedOrder::hero(Order::Move {
                target: Target::None,
            })]
        } else {
            vec![]
        };
        assert_eq!(turn.bots[0].orders, expected);
    }
}

#[test]
fn item_controller_damage_from_an_earlier_use_does_not_apply_a_new_item_request() {
    use bota_proto::{Inflictor, ItemId, ItemSlot, Target};
    let (mut local, mut sessions, target) =
        support::stationary_creep_encounter(HeroId::SHADOW_FIEND);
    step(
        &mut local,
        &mut sessions,
        &[
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Item {
                    item: ItemId::BLOOD_GRENADE,
                },
            }),
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Item {
                    item: ItemId::BLOOD_GRENADE,
                },
            }),
        ],
    );
    let own = sessions[0].own_hero().unwrap().id;
    let at = sessions[0].current().unwrap().unit(target).unwrap().pos;
    step(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Use {
            slot: ItemSlot(0),
            target: Target::Pos(at),
        })],
    );
    let damage = |session: &bota_client_core::ClientSession, after: u32| {
        session.recent_events().iter().any(|event|event.tick>after && matches!(event.kind,EventKind::Damaged {source:Some(source),inflictor:Some(Inflictor::Item(ItemId::BLOOD_GRENADE)),..} if source==own))
    };
    let thrown = sessions[0].current().unwrap().tick;
    for _ in 0..60 {
        if damage(&sessions[0], thrown) {
            break;
        }
        step(&mut local, &mut sessions, &[]);
    }
    assert!(
        damage(&sessions[0], thrown),
        "the first grenade really hits"
    );
    let mut lua=host(b"local done=false function Think() local b=GetBot();if not done then b:Action_UseAbilityOnLocation(b:GetItemInSlot(0),b:GetLocation());b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256,0,0));done=true else assert(b:GetCurrentActionType()==BOT_ACTION_TYPE_USE_ABILITY);assert(b:NumQueuedActions()==1) end end");
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty());
    assert!(matches!(first.bots[0].orders[0].order, Order::Use { .. }));
    let started = sessions[0].current().unwrap().tick;
    let mut later_damage = false;
    for _ in 0..60 {
        // Hold the new request: only the first grenade's ongoing damage reaches this stream.
        step(&mut local, &mut sessions, &[]);
        later_damage |= damage(&sessions[0], started);
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert!(turn.bots[0].orders.is_empty());
    }
    assert!(
        later_damage,
        "ongoing damage cannot be mistaken for applying the new request"
    );
}

#[test]
fn item_controller_a_slot_restored_after_refusal_does_not_revive_the_failed_request() {
    use bota_proto::{ItemId, ItemSlot, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    step(
        &mut local,
        &mut sessions,
        &[
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Item {
                    item: ItemId::MANGO,
                },
            }),
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Item {
                    item: ItemId::IRON_BRANCH,
                },
            }),
        ],
    );
    let goal = sessions[0].own_hero().unwrap().pos + Vec2::from_ints(256, 0);
    let mut lua=host(b"local done=false function Think() if not done then local b=GetBot();b:Action_UseAbility(b:GetItemInSlot(0));b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256,0,0));done=true end end");
    let pending = lua.think(&sessions[0]).unwrap();
    assert!(pending.failures.is_empty());
    let swap = IssuedOrder::hero(Order::Swap {
        from: ItemSlot(0),
        to: ItemSlot(1),
    });
    step(&mut local, &mut sessions, &[swap]);
    let refused = step(&mut local, &mut sessions, &pending.bots[0].orders);
    assert!(refused[0].iter().any(|msg| matches!(
        msg,
        bota_proto::ServerMsg::OrderRejected {
            reason: bota_proto::RejectReason::WrongTargetKind,
            ..
        }
    )));
    step(&mut local, &mut sessions, &[swap]);
    assert_eq!(
        sessions[0].own_hero().unwrap().items[0].unwrap().id,
        ItemId::MANGO
    );
    let next = lua.think(&sessions[0]).unwrap();
    assert!(next.failures.is_empty());
    assert_eq!(
        next.bots[0].orders,
        vec![IssuedOrder::hero(Order::Move {
            target: Target::Pos(goal)
        })],
        "a real refusal remains terminal after the original slot contents return"
    );
}

#[test]
fn item_controller_received_applications_complete_without_locally_numbering_the_order() {
    use bota_proto::{ItemSlot, Target};
    let item = bota_core::item_by_key("item_phase_boots").unwrap();
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    step(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Item { item },
        })],
    );
    let goal = sessions[0].own_hero().unwrap().pos + Vec2::from_ints(256, 0);
    let mut lua=host(b"local done=false function Think() if not done then local b=GetBot();b:Action_UseAbility(b:GetItemInSlot(0));b:ActionQueue_MoveToLocation(b:GetLocation()+Vector(256,0,0));done=true end end");
    let first = lua.think(&sessions[0]).unwrap();
    assert!(first.failures.is_empty());
    assert_eq!(
        first.bots[0].orders[0].order,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None
        }
    );
    let mut sender = sessions[0].clone();
    let outgoing = sender.issue(first.bots[0].orders[0]).unwrap();
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[outgoing], &[]]).unwrap())
    {
        support::fold(session, stream);
    }
    assert!(
        sessions[0]
            .recent_events()
            .iter()
            .any(|event| matches!(event.kind,EventKind::ItemUsed {item:used,..} if used==item))
    );
    let next = lua.think(&sessions[0]).unwrap();
    assert!(next.failures.is_empty());
    assert_eq!(
        next.bots[0].orders,
        vec![IssuedOrder::hero(Order::Move {
            target: Target::Pos(goal)
        })],
        "observation-only embeddings retain successful applications too"
    );
}

#[test]
fn item_controller_an_unrelated_spell_ending_does_not_fail_a_pending_item_request() {
    use bota_proto::{AbilitySlot, ItemId, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    step(
        &mut local,
        &mut sessions,
        &[
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Item {
                    item: ItemId::MANGO,
                },
            }),
            IssuedOrder::hero(Order::Learn {
                slot: AbilitySlot(0),
            }),
        ],
    );
    step(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Cast {
            slot: AbilitySlot(0),
            target: Target::None,
        })],
    );
    assert!(
        sessions[0].own_hero().unwrap().cast.is_some(),
        "a real earlier spell is still winding up"
    );
    let mut lua=host(b"local done=false function Think() if not done then local bot=GetBot();bot:Action_UseAbility(bot:GetItemInSlot(0));bot:ActionQueue_MoveToLocation(bot:GetLocation()+Vector(256,0,0));done=true end end");
    let pending = lua.think(&sessions[0]).unwrap();
    assert!(pending.failures.is_empty());
    assert!(matches!(pending.bots[0].orders[0].order, Order::Use { .. }));
    for _ in 0..30 {
        // Hold the item request while the earlier spell completes normally.
        step(&mut local, &mut sessions, &[]);
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty());
        assert!(
            turn.bots[0].orders.is_empty(),
            "an unrelated cast ending is not a failed item request"
        );
    }
    assert!(sessions[0].own_hero().unwrap().cast.is_none());
}
