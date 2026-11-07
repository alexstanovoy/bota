use super::*;
const HANG_GUARD: Duration = Duration::from_secs(30);

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
        timeout: Duration::ZERO,
        last_tick: None,
        in_flight: false,
        late_ticks: 0,
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

mod support { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs")); }

#[test]
fn one_late_reply_is_discarded_without_retiring_the_worker() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let (requests, input) = mpsc::sync_channel::<Snapshot>(1);
    let (output, replies) = mpsc::sync_channel(1);
    let (release, blocked) = mpsc::channel();
    let worker = thread::spawn(move || {
        let old = input.recv().expect("first request");
        blocked.recv_timeout(HANG_GUARD).expect("release late request");
        if output.send(Ok(marker_turn(old.tick))).is_err() { return; }
        if let Ok(fresh) = input.recv_timeout(HANG_GUARD) { let _ = output.send(Ok(marker_turn(fresh.tick))); }
    });
    let mut native = NativeTeam { requests: Some(requests), replies: Some(replies), worker: Some(worker), team: Team::Radiant, seats: Vec::new(), timeout: Duration::from_millis(33), last_tick: None, in_flight: false, late_ticks: 0 };
    let first = native.think(&sessions[0]);
    let active = !native.retired();
    release.send(()).expect("release");
    let mut view = sessions[0].current().expect("view").clone(); view.tick += 1;
    let tick = view.tick;
    sessions[0].ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ()).expect("snapshot");
    sessions[0].ingest(bota_proto::ServerMsg::Events { tick, events: Vec::new() }, &mut ()).expect("events");
    let fresh = native.think(&sessions[0]);
    let recovered_streak = native.late_ticks;
    let worker = native.worker.take().expect("worker"); drop(native); worker.join().expect("worker exits");
    assert!(matches!(first, Err(NativeError::Deadline)));
    assert!(active, "a single 33ms miss must not retire the team");
    let fresh = fresh.expect("worker handles next tick");
    assert_eq!(recovered_streak, 0, "a timely tick resets the deadline streak");
    assert_eq!(fresh.bots[0].orders[0].order, bota_proto::Order::Move { target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(tick as i32, 0)) });
}

fn marker_turn(tick: u32) -> TeamTurn {
    TeamTurn { bots: vec![crate::BotTurn { slot: bota_proto::SlotId(0), orders: vec![bota_client_core::IssuedOrder::hero(bota_proto::Order::Move { target: bota_proto::Target::Pos(bota_proto::Vec2::from_ints(tick as i32, 0)) })] }], failures: Vec::new() }
}

#[test]
fn consecutive_late_ticks_retire_a_still_busy_worker_without_queueing_more_work() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let (requests, input) = mpsc::sync_channel::<Snapshot>(1);
    let (output, replies) = mpsc::sync_channel(1);
    let (release, blocked) = mpsc::channel();
    let worker = thread::spawn(move || {
        let request = input.recv().expect("request");
        blocked.recv_timeout(HANG_GUARD).expect("release");
        let _ = output.send(Ok(marker_turn(request.tick)));
        assert!(input.try_recv().is_err(), "only one request may be in flight");
    });
    let mut native = NativeTeam { requests: Some(requests), replies: Some(replies), worker: Some(worker), team: Team::Radiant, seats: Vec::new(), timeout: Duration::from_millis(33), last_tick: None, in_flight: false, late_ticks: 0 };
    let mut retired = Vec::new();
    for _ in 0..3 {
        assert!(matches!(native.think(&sessions[0]), Err(NativeError::Deadline)));
        retired.push(native.retired());
        let mut view = sessions[0].current().expect("view").clone(); view.tick += 1;
        let tick=view.tick;
        sessions[0].ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ()).expect("snapshot");
        sessions[0].ingest(bota_proto::ServerMsg::Events { tick, events:Vec::new() }, &mut ()).expect("events");
    }
    release.send(()).expect("release");
    let worker=native.worker.take().expect("worker"); drop(native); worker.join().expect("worker exits");
    assert_eq!(retired, vec![false,false,true]);
}
