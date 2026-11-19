#![cfg(all(feature = "native", not(target_arch = "wasm32")))]
mod support;
use bota_lua::{BotSeat, Keys, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};
#[test]
fn the_observation_history_query_does_not_replace_the_scripts_movement_intent() {
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new_measured(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        bota_lua::native::NativeConfig::new(
            Team::Radiant,
            vec![BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
            Keys {
                hash: [17, 19],
                random: 23,
            },
        )
        .limits,
        Keys {
            hash: [17, 19],
            random: 23,
        },
        true,
    )
    .unwrap();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"stability_fixture",b"local frame=0 function Think() frame=frame+1;local b=GetBot();assert(type(b.GetMovementDirectionStability)=='function','stability API absent');local stability=b:GetMovementDirectionStability();if frame==1 then assert(stability==nil,'missing history must not be invented') end if frame>40 then assert(type(stability)=='number','fully observed history must answer') end if stability~=nil then assert(stability>=0 and stability<=1) end b:Action_MoveDirectly(Vector(-6000,-6500,0)) end").unwrap();
    let start = sessions[0].own_hero().unwrap().pos;
    for _ in 0..90 {
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(turn.hook_activity.native, 0);
        assert_eq!(turn.bots[0].orders.len(), 1);
        assert!(
            matches!(turn.bots[0].orders[0].order,bota_proto::Order::Move {target:bota_proto::Target::Pos(at)} if at==bota_proto::Vec2::from_ints(3216,2716))
        );
        let outgoing: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .map(|&issued| sessions[0].issue(issued).unwrap())
            .collect();
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&outgoing, &[]]).unwrap())
        {
            support::fold(session, stream);
        }
    }
    assert_ne!(sessions[0].own_hero().unwrap().pos, start);
}

#[test]
fn hidden_movement_differences_cannot_enter_a_seats_stability_query() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, Order, Target, Vec2};
    fn setup() -> (
        bota_server::local::LocalMatch,
        Vec<bota_client_core::ClientSession>,
        LuaTeam,
    ) {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        let positions = [Vec2::from_ints(9216, 9216), Vec2::from_ints(9500, 9216)];
        let commands: Vec<_> = (0..2)
            .map(|at| {
                sessions[at]
                    .issue(IssuedOrder::hero(Order::Cheat {
                        cheat: Cheat::Teleport { at: positions[at] },
                    }))
                    .unwrap()
            })
            .collect();
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&[commands[0]], &[commands[1]]]).unwrap())
        {
            support::fold(session, stream);
        }
        let keys = Keys {
            hash: [17, 19],
            random: 23,
        };
        let seats = [BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }];
        let limits =
            bota_lua::native::NativeConfig::new(Team::Radiant, seats.to_vec(), keys).limits;
        let mut lua = LuaTeam::new(Team::Radiant, &seats, limits, keys).unwrap();
        lua.add_script(SlotId(0),ScriptKind::Takeover,"hidden_history",b"local enemy local frame=0 function Think() frame=frame+1;if not enemy then enemy=GetUnitList(UNIT_LIST_ENEMY_HEROES)[1];assert(enemy) end local value=enemy:GetMovementDirectionStability();if frame>31 and frame<=40 then assert(type(value)=='number','visible window must fill') elseif frame>40 then assert(value==nil,'hidden or newly visible history must be unavailable') end GetBot():Action_MoveDirectly(Vector(-6000,-6500,0)) end").unwrap();
        (local, sessions, lua)
    }
    let (mut a, mut sa, mut la) = setup();
    let (mut b, mut sb, mut lb) = setup();
    let enemy = sa[1].own_hero().unwrap().id;
    for _ in 0..40 {
        let aa = la.think(&sa[0]).unwrap();
        let bb = lb.think(&sb[0]).unwrap();
        assert!(aa.failures.is_empty(), "{:?}", aa.failures);
        assert!(bb.failures.is_empty(), "{:?}", bb.failures);
        let oa: Vec<_> = aa.bots[0]
            .orders
            .iter()
            .map(|&order| sa[0].issue(order).unwrap())
            .collect();
        let ob: Vec<_> = bb.bots[0]
            .orders
            .iter()
            .map(|&order| sb[0].issue(order).unwrap())
            .collect();
        for (session, stream) in sa.iter_mut().zip(a.step(&[&oa, &[]]).unwrap()) {
            support::fold(session, stream);
        }
        for (session, stream) in sb.iter_mut().zip(b.step(&[&ob, &[]]).unwrap()) {
            support::fold(session, stream);
        }
    }
    for (local, sessions) in [(&mut a, &mut sa), (&mut b, &mut sb)] {
        let hide = sessions[1]
            .issue(IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Teleport {
                    at: Vec2::from_ints(16000, 16000),
                },
            }))
            .unwrap();
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&[], &[hide]]).unwrap())
        {
            support::fold(session, stream);
        }
        assert!(sessions[0].current().unwrap().unit(enemy).is_none());
    }
    let mut hidden_differed = false;
    for tick in 0..40 {
        let aa = la.think(&sa[0]).unwrap();
        let bb = lb.think(&sb[0]).unwrap();
        assert!(aa.failures.is_empty(), "{:?}", aa.failures);
        assert!(bb.failures.is_empty(), "{:?}", bb.failures);
        assert_eq!(aa.bots[0].orders, bb.bots[0].orders);
        let own_a: Vec<_> = aa.bots[0]
            .orders
            .iter()
            .map(|&order| sa[0].issue(order).unwrap())
            .collect();
        let own_b: Vec<_> = bb.bots[0]
            .orders
            .iter()
            .map(|&order| sb[0].issue(order).unwrap())
            .collect();
        let hidden = sb[1]
            .issue(IssuedOrder::hero(Order::Move {
                target: Target::Pos(Vec2::from_ints(
                    if tick < 20 { 15000 } else { 17000 },
                    16000,
                )),
            }))
            .unwrap();
        for (session, stream) in sa.iter_mut().zip(a.step(&[&own_a, &[]]).unwrap()) {
            support::fold(session, stream);
        }
        for (session, stream) in sb.iter_mut().zip(b.step(&[&own_b, &[hidden]]).unwrap()) {
            support::fold(session, stream);
        }
        hidden_differed |= sa[1].own_hero().unwrap().pos != sb[1].own_hero().unwrap().pos;
    }
    assert!(hidden_differed, "the hidden movement paths really differ");
    for (local, sessions, lua) in [(&mut a, &mut sa, &mut la), (&mut b, &mut sb, &mut lb)] {
        let near = sessions[0].own_hero().unwrap().pos + Vec2::from_ints(224, 0);
        let show = sessions[1]
            .issue(IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Teleport { at: near },
            }))
            .unwrap();
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&[], &[show]]).unwrap())
        {
            support::fold(session, stream);
        }
        assert!(sessions[0].current().unwrap().unit(enemy).is_some());
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    }
}

#[test]
fn the_entity_tracker_restarts_observed_history_after_a_death_and_respawn_gap() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, Order};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let hero = sessions[0].own_hero().unwrap().id;
    fn advance(
        local: &mut bota_server::local::LocalMatch,
        sessions: &mut [bota_client_core::ClientSession],
        orders: &[IssuedOrder],
    ) {
        let outgoing: Vec<_> = orders
            .iter()
            .map(|&order| sessions[0].issue(order).unwrap())
            .collect();
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&outgoing, &[]]).unwrap())
        {
            support::fold(session, stream);
        }
    }
    for _ in 0..31 {
        advance(&mut local, &mut sessions, &[]);
    }
    assert_eq!(
        sessions[0]
            .entity(hero)
            .unwrap()
            .direction_history
            .stability(30),
        Some(1.0)
    );
    advance(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Suicide,
        })],
    );
    assert!(
        sessions[0].entity(hero).unwrap().last_death.is_some(),
        "the shared tracker folded the death event"
    );
    for _ in 0..3 {
        advance(&mut local, &mut sessions, &[]);
        assert_eq!(
            sessions[0]
                .entity(hero)
                .unwrap()
                .direction_history
                .stability(30),
            None
        );
    }
    advance(
        &mut local,
        &mut sessions,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Respawn,
        })],
    );
    let track = sessions[0].entity(hero).unwrap();
    assert!(track.visible);
    assert!(
        track.last_death.is_none(),
        "the observed respawn traversed the tracker reset branch"
    );
    assert_eq!(track.direction_history.stability(30), None);
    for _ in 0..29 {
        advance(&mut local, &mut sessions, &[]);
    }
    assert_eq!(
        sessions[0]
            .entity(hero)
            .unwrap()
            .direction_history
            .stability(30),
        None
    );
    advance(&mut local, &mut sessions, &[]);
    assert_eq!(
        sessions[0]
            .entity(hero)
            .unwrap()
            .direction_history
            .stability(30),
        Some(1.0)
    );
}

#[test]
fn the_entity_tracker_keeps_consecutive_positions_across_a_death_and_immediate_respawn() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, Order};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let hero = sessions[0].own_hero().unwrap().id;
    for _ in 0..31 {
        for (session, stream) in sessions.iter_mut().zip(local.step(&[&[], &[]]).unwrap()) {
            support::fold(session, stream);
        }
    }
    assert!(
        sessions[0]
            .entity(hero)
            .unwrap()
            .direction_history
            .stability(30)
            .is_some()
    );
    let commands: Vec<_> = [Cheat::Suicide, Cheat::Respawn]
        .into_iter()
        .map(|cheat| {
            sessions[0]
                .issue(IssuedOrder::hero(Order::Cheat { cheat }))
                .unwrap()
        })
        .collect();
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&commands, &[]]).unwrap())
    {
        support::fold(session, stream);
    }
    assert_eq!(sessions[0].own_hero().unwrap().id, hero);
    assert!(
        sessions[0]
            .entity(hero)
            .unwrap()
            .direction_history
            .stability(30)
            .is_some(),
        "death is not an observation gap"
    );
    for (session, stream) in sessions.iter_mut().zip(local.step(&[&[], &[]]).unwrap()) {
        support::fold(session, stream);
    }
    let track = sessions[0].entity(hero).unwrap();
    assert!(track.last_death.is_none());
    assert!(
        track.direction_history.stability(30).is_some(),
        "the respawn tracker branch must retain consecutive observations"
    );
}
