use super::*;
const HANG_GUARD: Duration = Duration::from_secs(30);

#[test]
fn a_discarded_courier_turn_retries_its_unpublished_request() {
    use bota_proto::{HeroId, Order, SlotId};
    let (mut local, mut sessions) = support::courier_delivery_setup();
    let (requests, input) = mpsc::sync_channel(1);
    let (output, replies) = mpsc::sync_channel(1);
    let (ready, startup) = mpsc::channel();
    let (entered, waiting) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let mut config = NativeConfig::new(
        Team::Radiant,
        vec![BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Keys {
            hash: [17, 19],
            random: 23,
        },
    );
    config.hero_selection = HeroSelection::Fixed;
    let worker = thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(move || {
            let (mut lua, _) =
                build(config, support::courier_modules(false), Some(&input)).expect("setup");
            ready.send(()).expect("ready");
            let mut held = false;
            run_with_reply_hook(&mut lua, input, output, |reply| {
                if !held
                    && reply.as_ref().is_ok_and(|turn| {
                        turn.bots.iter().any(|bot| {
                            bot.orders.iter().any(|order| {
                                order.unit.is_some() && matches!(order.order, Order::Cast { .. })
                            })
                        })
                    })
                {
                    held = true;
                    entered.send(()).expect("courier reply");
                    blocked.recv_timeout(HANG_GUARD).expect("release reply");
                }
            });
        })
        .expect("worker");
    startup.recv_timeout(HANG_GUARD).expect("startup");
    let mut native = NativeTeam {
        requests: Some(requests),
        replies: Some(replies),
        worker: Some(worker),
        team: Team::Radiant,
        seats: Vec::new(),
        deterministic: false,
        timeout: Duration::from_millis(33),
        last_tick: None,
        in_flight: false,
        late_ticks: 0,
        deadline_ticks: 0,
        discard_previous: false,
        statistics: None,
    };
    assert!(matches!(
        native.think(&sessions[0]),
        Err(NativeError::Deadline)
    ));
    assert_eq!(native.deadline_ticks(), 1);
    waiting
        .recv_timeout(HANG_GUARD)
        .expect("withheld courier result");
    release.send(()).expect("release");
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&[&[], &[]]).expect("next tick"))
    {
        support::fold(session, stream);
    }
    // The first deadline is real; retry timing is not the contract under test.
    native.timeout = Duration::from_secs(1);
    let retried = native.think(&sessions[0]).expect("retry");
    assert_eq!(
        native.deadline_ticks(),
        1,
        "recovery must not erase an earlier missed deadline"
    );
    assert!(retried.failures.is_empty(), "{:?}", retried.failures);
    assert!(
        retried.bots[0]
            .orders
            .iter()
            .any(|order| order.unit.is_some() && matches!(order.order, Order::Cast { .. }))
    );
    let worker = native.worker.take().expect("worker");
    drop(native);
    worker.join().expect("exit");
}

#[test]
fn lifecycle_drop_returns_before_an_inflight_worker_is_released() {
    let (release, blocked) = mpsc::channel();
    let (entered, waiting) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let worker = thread::spawn(move || {
        entered.send(()).expect("entered");
        blocked.recv().expect("release");
        done.send(()).expect("done");
    });
    waiting.recv_timeout(HANG_GUARD).expect("worker blocked");
    let (requests, _) = mpsc::sync_channel(1);
    let (_, replies) = mpsc::sync_channel(1);
    let native = NativeTeam {
        requests: Some(requests),
        replies: Some(replies),
        worker: Some(worker),
        team: Team::Radiant,
        seats: Vec::new(),
        deterministic: false,
        timeout: Duration::ZERO,
        last_tick: None,
        in_flight: false,
        late_ticks: 0,
        deadline_ticks: 0,
        discard_previous: false,
        statistics: None,
    };
    let (returned, dropped) = mpsc::channel();
    let caller = thread::spawn(move || {
        drop(native);
        returned.send(()).expect("caller returned");
    });
    let completed = dropped.recv_timeout(HANG_GUARD).is_ok();
    release.send(()).expect("release");
    finished.recv_timeout(HANG_GUARD).expect("worker exits");
    caller.join().expect("caller exits");
    assert!(
        completed,
        "dropping a retired worker waited for its computation"
    );
}

#[test]
fn lifecycle_startup_deadline_returns_before_setup_is_released() {
    let (release, blocked) = mpsc::channel();
    let (entered, waiting) = mpsc::channel();
    let (returned, result) = mpsc::channel();
    let caller = thread::spawn(move || {
        let mut config = NativeConfig::new(
            Team::Radiant,
            vec![BotSeat {
                slot: bota_proto::SlotId(0),
                hero: bota_proto::HeroId::SHADOW_FIEND,
            }],
            Keys {
                hash: [17, 19],
                random: 23,
            },
        );
        config.startup_timeout = Duration::from_millis(1);
        let outcome = NativeTeam::start(config, ModuleBank::new(Limits::DEFAULT), move || {
            entered.send(()).expect("entered");
            blocked.recv().expect("release");
        });
        returned
            .send(matches!(outcome, Err(NativeError::Deadline)))
            .expect("result");
    });
    waiting.recv_timeout(HANG_GUARD).expect("setup blocked");
    let completed = result.recv_timeout(HANG_GUARD).ok();
    release.send(()).expect("release");
    caller.join().expect("caller exits");
    assert_eq!(
        completed,
        Some(true),
        "startup timeout synchronously joined setup"
    );
}

#[test]
fn native_limits_cannot_disable_fuel_enforcement() {
    let mut config = NativeConfig::new(
        Team::Radiant,
        vec![BotSeat {
            slot: bota_proto::SlotId(0),
            hero: bota_proto::HeroId::SHADOW_FIEND,
        }],
        Keys {
            hash: [17, 19],
            random: 23,
        },
    );
    config.limits.fuel_per_call = u64::MAX;
    assert!(matches!(
        NativeTeam::new(config, ModuleBank::new(Limits::DEFAULT)),
        Err(NativeError::Setup(_))
    ));
}

mod support {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
}

#[test]
fn one_late_reply_is_discarded_without_retiring_the_worker() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let (requests, input) = mpsc::sync_channel::<Request>(1);
    let (output, replies) = mpsc::sync_channel(1);
    let (release, blocked) = mpsc::channel();
    let worker = thread::spawn(move || {
        let old = input.recv().expect("first request");
        blocked
            .recv_timeout(HANG_GUARD)
            .expect("release late request");
        if output.send(Ok(marker_turn(old.snapshot.tick))).is_err() {
            return;
        }
        if let Ok(fresh) = input.recv_timeout(HANG_GUARD) {
            let _ = output.send(Ok(marker_turn(fresh.snapshot.tick)));
        }
    });
    let mut native = NativeTeam {
        requests: Some(requests),
        replies: Some(replies),
        worker: Some(worker),
        team: Team::Radiant,
        seats: Vec::new(),
        deterministic: false,
        timeout: Duration::from_millis(33),
        last_tick: None,
        in_flight: false,
        late_ticks: 0,
        deadline_ticks: 0,
        discard_previous: false,
        statistics: None,
    };
    let first = native.think(&sessions[0]);
    let active = !native.retired();
    release.send(()).expect("release");
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    let tick = view.tick;
    sessions[0]
        .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
        .expect("snapshot");
    sessions[0]
        .ingest(
            bota_proto::ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    let fresh = native.think(&sessions[0]);
    let recovered_streak = native.late_ticks;
    let worker = native.worker.take().expect("worker");
    drop(native);
    worker.join().expect("worker exits");
    assert!(matches!(first, Err(NativeError::Deadline)));
    assert!(active, "a single 33ms miss must not retire the team");
    let fresh = fresh.expect("worker handles next tick");
    assert_eq!(
        recovered_streak, 0,
        "a timely tick resets the deadline streak"
    );
    assert_eq!(
        fresh.bots[0].orders[0].order,
        bota_proto::Order::Move {
            target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(tick as i32, 0))
        }
    );
}

fn marker_turn(tick: u32) -> TeamTurn {
    TeamTurn {
        statistics: None,
        bots: vec![crate::BotTurn {
            slot: bota_proto::SlotId(0),
            orders: vec![bota_client_core::IssuedOrder::hero(
                bota_proto::Order::Move {
                    target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(tick as i32, 0)),
                },
            )],
        }],
        failures: Vec::new(),
        hook_activity: crate::HookActivity::default(),
    }
}

#[test]
fn consecutive_late_ticks_retire_a_still_busy_worker_without_queueing_more_work() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let (requests, input) = mpsc::sync_channel::<Request>(1);
    let (output, replies) = mpsc::sync_channel(1);
    let (release, blocked) = mpsc::channel();
    let worker = thread::spawn(move || {
        let request = input.recv().expect("request");
        blocked.recv_timeout(HANG_GUARD).expect("release");
        let _ = output.send(Ok(marker_turn(request.snapshot.tick)));
        assert!(
            input.try_recv().is_err(),
            "only one request may be in flight"
        );
    });
    let mut native = NativeTeam {
        requests: Some(requests),
        replies: Some(replies),
        worker: Some(worker),
        team: Team::Radiant,
        seats: Vec::new(),
        deterministic: false,
        timeout: Duration::from_millis(33),
        last_tick: None,
        in_flight: false,
        late_ticks: 0,
        deadline_ticks: 0,
        discard_previous: false,
        statistics: None,
    };
    let mut retired = Vec::new();
    for _ in 0..3 {
        assert!(matches!(
            native.think(&sessions[0]),
            Err(NativeError::Deadline)
        ));
        retired.push(native.retired());
        let mut view = sessions[0].current().expect("view").clone();
        view.tick += 1;
        let tick = view.tick;
        sessions[0]
            .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
            .expect("snapshot");
        sessions[0]
            .ingest(
                bota_proto::ServerMsg::Events {
                    tick,
                    events: Vec::new(),
                },
                &mut (),
            )
            .expect("events");
    }
    release.send(()).expect("release");
    let worker = native.worker.take().expect("worker");
    drop(native);
    worker.join().expect("worker exits");
    assert_eq!(retired, vec![false, false, true]);
}

#[test]
fn a_discarded_completion_turn_requeues_stop_until_a_turn_is_accepted() {
    use bota_proto::{HeroId, Order, SlotId, Target};
    let (mut local, mut sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let source = sessions[0].own_hero().expect("hero").id;
    let (requests, input) = mpsc::sync_channel(1);
    let (output, replies) = mpsc::sync_channel(1);
    let (ready, startup) = mpsc::channel();
    let (entered, waiting) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    modules.insert("bots/bot_nevermore", b"local issued=false function Think() if not issued then local b=GetBot() local c=b:GetNearbyCreeps(1600,true) assert(#c>0) b:Action_AttackUnit(c[1],true) issued=true end end").expect("script");
    let config = NativeConfig::new(
        Team::Radiant,
        vec![BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Keys {
            hash: [17, 19],
            random: 23,
        },
    );
    let worker = thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(move || {
            let (mut lua, _) = build(config, modules, Some(&input)).expect("worker setup");
            ready.send(()).expect("ready");
            let mut held = false;
            run_with_reply_hook(&mut lua, input, output, |reply| {
                if !held
                    && reply.as_ref().is_ok_and(|turn| {
                        turn.bots.iter().any(|bot| {
                            bot.orders.iter().any(|order| {
                                order.order
                                    == (Order::Move {
                                        target: Target::None,
                                    })
                            })
                        })
                    })
                {
                    held = true;
                    entered.send(()).expect("completion reached");
                    blocked
                        .recv_timeout(HANG_GUARD)
                        .expect("release completion");
                }
            });
        })
        .expect("thread");
    startup.recv_timeout(HANG_GUARD).expect("ready");
    let mut native = NativeTeam {
        requests: Some(requests),
        replies: Some(replies),
        worker: Some(worker),
        team: Team::Radiant,
        seats: Vec::new(),
        deterministic: false,
        timeout: Duration::from_millis(33),
        last_tick: None,
        in_flight: false,
        late_ticks: 0,
        deadline_ticks: 0,
        discard_previous: false,
        statistics: None,
    };
    let mut late_completion = false;
    let mut stops = 0;
    let mut accepted_stop = None;
    for _ in 0..600 {
        let orders = match native.think(&sessions[0]) {
            Ok(turn) => {
                assert!(turn.failures.is_empty());
                turn.bots
                    .into_iter()
                    .flat_map(|bot| bot.orders)
                    .collect::<Vec<_>>()
            }
            Err(NativeError::Deadline) => {
                if waiting.try_recv().is_ok() {
                    late_completion = true;
                    release.send(()).expect("release");
                }
                Vec::new()
            }
            Err(error) => panic!("unexpected worker error: {error}"),
        };
        let stop_count = orders
            .iter()
            .filter(|order| {
                order.order
                    == (Order::Move {
                        target: Target::None,
                    })
            })
            .count();
        stops += stop_count;
        if stop_count > 0 {
            accepted_stop.get_or_insert(sessions[0].current().expect("view").tick);
        }
        let orders: Vec<_> = orders
            .into_iter()
            .map(|order| sessions[0].issue(order).expect("order"))
            .collect();
        let streams = local.step(&[&orders, &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
        if accepted_stop.is_some_and(|tick| sessions[0].current().expect("view").tick > tick + 60) {
            break;
        }
    }
    let worker = native.worker.take().expect("worker");
    drop(native);
    worker.join().expect("exit");
    assert!(
        late_completion,
        "the actual completion reply was withheld beyond33ms"
    );
    assert_eq!(
        stops, 1,
        "the discarded completion must be retried exactly once"
    );
    assert!(
        sessions[0]
            .entity(source)
            .and_then(|unit| unit.last_attack_launched)
            .is_some()
    );
}
