mod support;

use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{EntityId, EventKind, HeroId, Order, SlotId, Target, Team};
use bota_server::local::LocalMatch;
use std::collections::BTreeSet;


fn attack_one_creep_then_stop(hero: HeroId, ranged: bool) {
    let (mut local, mut sessions, target) = support::durable_creep_encounter(hero);
    let own = sessions[0]
        .own_player()
        .expect("player")
        .unit
        .expect("hero");
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "once", b"local issued=false function Think() if not issued then local b=GetBot() local c=b:GetNearbyCreeps(1600,true) assert(#c>0) b:Action_AttackUnit(c[1],true) issued=true end end").expect("script");
    let mut projectiles = BTreeSet::new();
    let mut melee_attacks = 0;
    let mut stops = 0;
    let mut stopped_at = None;
    let mut last_tick = sessions[0].current().expect("view").tick;
    for _ in 0..600 {
        let turn = lua.think(&sessions[0]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        for issued in &turn.bots[0].orders {
            if issued.order
                == (Order::Move {
                    target: Target::None,
                })
            {
                stops += 1;
                stopped_at = Some(last_tick);
            }
        }
        let orders: Vec<_> = turn.bots[0]
            .orders
            .iter()
            .map(|&issued| sessions[0].issue(issued).expect("order"))
            .collect();
        let streams = local.step(&[&orders, &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
        for projectile in &sessions[0].current().expect("view").projectiles {
            if projectile.source == Some(own) && projectile.inflictor.is_none() {
                projectiles.insert(projectile.id);
            }
        }
        for event in sessions[0]
            .recent_events()
            .iter()
            .filter(|event| event.tick > last_tick)
        {
            match event.kind {
                EventKind::Damaged {
                    source: Some(source),
                    target: hit,
                    inflictor: None,
                    removal: false,
                    ..
                }
                | EventKind::Missed {
                    source: Some(source),
                    target: hit,
                } if source == own && hit == target => melee_attacks += 1,
                _ => {}
            }
        }
        last_tick = sessions[0].current().expect("view").tick;
        if stopped_at.is_some_and(|tick| last_tick > tick + 150) {
            break;
        }
    }
    assert_eq!(stops, 1, "the once action emits exactly one Stop");
    assert!(
        sessions[0].own_hero().is_some(),
        "the hero remained alive after stopping"
    );
    assert_eq!(
        if ranged {
            projectiles.len()
        } else {
            melee_attacks
        },
        1,
        "one attack, with no later repeat"
    );
}

#[test]
fn shadow_fiend_launches_one_attack_at_a_creep_then_stops() {
    attack_one_creep_then_stop(HeroId::SHADOW_FIEND, true);
}

#[test]
fn pudge_lands_one_melee_attack_at_a_creep_then_stops() {
    attack_one_creep_then_stop(HeroId::PUDGE, false);
}

fn once_script() -> LuaTeam {
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
    .expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "gap_once", b"local issued=false function Think() if not issued then local b=GetBot() local c=b:GetNearbyCreeps(1600,true) assert(#c>0) b:Action_AttackUnit(c[1],true) issued=true end end").expect("script");
    lua
}

fn step(
    local: &mut LocalMatch,
    sessions: &mut [ClientSession],
    orders: &[bota_proto::OutgoingOrder],
) {
    let streams = local.step(&[orders, &[]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
}

fn issued(session: &mut ClientSession, turn: bota_lua::TeamTurn) -> Vec<bota_proto::OutgoingOrder> {
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    turn.bots[0]
        .orders
        .iter()
        .map(|&order| session.issue(order).expect("order"))
        .collect()
}

fn an_attack_arrived(
    session: &ClientSession,
    source: EntityId,
    target: EntityId,
    after: u32,
) -> bool {
    session.recent_events().iter().any(|event| {
        event.tick > after
            && match event.kind {
                EventKind::Damaged {
                    source: Some(by),
                    target: on,
                    inflictor: None,
                    removal: false,
                    ..
                }
                | EventKind::Missed {
                    source: Some(by),
                    target: on,
                } => by == source && on == target,
                _ => false,
            }
    })
}

#[test]
fn attack_once_resolves_a_missed_snapshot_gap_from_the_retained_launch() {
    attack_once_think_gap(false);
}

#[test]
fn attack_once_retains_a_delivered_launch_after_unrelated_events_evict_recent_history() {
    attack_once_think_gap(true);
}

fn attack_once_think_gap(evict: bool) {
    let (mut local, mut sessions, target) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let source = sessions[0]
        .own_player()
        .expect("player")
        .unit
        .expect("hero");
    let mut lua = once_script();
    let started = sessions[0].current().expect("view").tick;
    let mut delayed = sessions[0].clone();
    let mut skipped = Vec::new();
    let turn = lua.think(&sessions[0]).expect("request");
    let orders = issued(&mut sessions[0], turn);
    let mut arrived = false;
    for index in 0..300 {
        if an_attack_arrived(&sessions[0], source, target, started) {
            arrived = true;
            break;
        }
        let streams = local
            .step(&[if index == 0 { &orders } else { &[] }, &[]])
            .expect("step");
        for message in &streams[0] {
            if let bota_proto::ServerMsg::Events { tick, events } = message {
                skipped.push(bota_proto::SkippedTick {
                    tick: *tick,
                    events: events.clone(),
                });
            }
        }
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
    assert!(arrived, "one attack arrived while no thinks ran");
    assert!(
        !sessions[0]
            .current()
            .expect("view")
            .projectiles
            .iter()
            .any(|shot| shot.source == Some(source) && shot.inflictor.is_none())
    );
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    view.skipped_ticks = skipped;
    let tick = view.tick;
    delayed
        .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
        .expect("missed snapshots with retained events");
    delayed
        .ingest(
            bota_proto::ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("completed current tick");
    assert!(delayed.skipped_ticks() > 0);
    assert!(
        delayed
            .entity(source)
            .expect("tracked attacker")
            .last_attack_launched
            .is_some_and(|launch| launch.tick > started)
    );
    if evict {
        let mut view = delayed.current().expect("view").clone();
        view.tick += 1;
        view.skipped_ticks.clear();
        let tick = view.tick;
        delayed
            .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
            .expect("snapshot");
        delayed
            .ingest(
                bota_proto::ServerMsg::Events {
                    tick,
                    events: vec![
                        EventKind::TreeGrew {
                            at: bota_proto::Vec2::from_ints(9216, 9216)
                        };
                        bota_client_core::MAX_RECENT_EVENTS
                    ],
                },
                &mut (),
            )
            .expect("unrelated events");
        assert!(
            !delayed
                .recent_events()
                .iter()
                .any(|event| matches!(event.kind, EventKind::AttackLaunched { .. })),
            "launch observations stay outside recent history"
        );
    }
    let turn = lua.think(&delayed).expect("resume");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(
        turn.bots[0].orders.iter().any(|order| order.order
            == (Order::Move {
                target: Target::None
            })),
        "a completed shot is not repeated after the gap"
    );
}

#[test]
fn an_older_inflight_shot_cannot_complete_a_new_once_request() {
    let (mut local, mut sessions, target) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let source = sessions[0]
        .own_player()
        .expect("player")
        .unit
        .expect("hero");
    let old = sessions[0]
        .issue(IssuedOrder::hero(Order::Attack {
            target: Target::Unit(target),
        }))
        .expect("old attack");
    step(&mut local, &mut sessions, &[old]);
    let mut airborne = false;
    for _ in 0..300 {
        if sessions[0]
            .current()
            .expect("view")
            .projectiles
            .iter()
            .any(|shot| shot.source == Some(source) && shot.inflictor.is_none())
        {
            airborne = true;
            break;
        }
        step(&mut local, &mut sessions, &[]);
    }
    assert!(airborne, "an older shot is in flight");
    let mut lua = once_script();
    let requested = sessions[0].current().expect("view").tick;
    let turn = lua.think(&sessions[0]).expect("new request");
    let orders = issued(&mut sessions[0], turn);
    step(&mut local, &mut sessions, &orders);
    for _ in 0..60 {
        if an_attack_arrived(&sessions[0], source, target, requested) {
            break;
        }
        step(&mut local, &mut sessions, &[]);
    }
    assert!(an_attack_arrived(&sessions[0], source, target, requested));
    let turn = lua.think(&sessions[0]).expect("older impact");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(
        turn.bots[0].orders.is_empty(),
        "the older attack is not the requested attack"
    );
    let mut launched = false;
    for _ in 0..120 {
        if sessions[0]
            .entity(source)
            .and_then(|track| track.last_attack_launched)
            .is_some_and(|launch| launch.tick > requested)
        {
            launched = true;
            break;
        }
        step(&mut local, &mut sessions, &[]);
    }
    assert!(launched, "the requested new attack launched");
    let turn = lua.think(&sessions[0]).expect("new launch");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert!(turn.bots[0].orders.iter().any(|order| order.order
        == (Order::Move {
            target: Target::None
        })));
}

#[test]
fn a_continuous_attack_replaces_an_earlier_once_request() {
    let (mut local, mut sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let source = sessions[0]
        .own_player()
        .expect("player")
        .unit
        .expect("hero");
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
    .expect("team");
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "replace_once",
        br#"
        local issued=false
        function Think()
            if not issued then
                local bot=GetBot()
                local target=bot:GetNearbyCreeps(1600,true)[1]
                assert(target)
                bot:Action_AttackUnit(target,true)
                bot:Action_AttackUnit(target,false)
                issued=true
            end
        end
    "#,
    )
    .expect("script");
    let started = sessions[0].current().expect("view").tick;
    let mut launches = BTreeSet::new();
    for _ in 0..600 {
        let turn = lua.think(&sessions[0]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert!(
            !turn.bots[0].orders.iter().any(|order| order.order
                == (Order::Move {
                    target: Target::None
                })),
            "a replaced once request cannot stop the later continuous attack"
        );
        let orders = issued(&mut sessions[0], turn);
        step(&mut local, &mut sessions, &orders);
        if let Some(observed) = sessions[0]
            .entity(source)
            .and_then(|track| track.last_attack_launched)
            .filter(|observed| observed.tick > started)
        {
            launches.insert(observed.tick);
        }
        if launches.len() >= 2 {
            break;
        }
    }
    assert!(
        launches.len() >= 2,
        "the replacement remains a continuous attack"
    );
}
