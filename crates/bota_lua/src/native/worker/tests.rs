use super::*;
const HANG_GUARD: Duration = Duration::from_secs(30);

#[test]
fn lifecycle_drop_returns_before_an_inflight_worker_is_released() {
    let (release, blocked) = mpsc::channel();
    let (entered, waiting) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let worker = thread::spawn(move || { entered.send(()).expect("entered"); blocked.recv().expect("release"); done.send(()).expect("done"); });
    waiting.recv_timeout(HANG_GUARD).expect("worker blocked");
    let (requests, _) = mpsc::sync_channel(1);
    let (_, replies) = mpsc::sync_channel(1);
    let native = NativeTeam { requests: Some(requests), replies: Some(replies), worker: Some(worker), team: Team::Radiant, seats: Vec::new(), timeout: Duration::ZERO, last_tick: None };
    let (returned, dropped) = mpsc::channel();
    let caller = thread::spawn(move || { drop(native); returned.send(()).expect("caller returned"); });
    let completed = dropped.recv_timeout(HANG_GUARD).is_ok();
    release.send(()).expect("release");
    finished.recv_timeout(HANG_GUARD).expect("worker exits");
    caller.join().expect("caller exits");
    assert!(completed, "dropping a retired worker waited for its computation");
}

#[test]
fn lifecycle_startup_deadline_returns_before_setup_is_released() {
    let (release, blocked) = mpsc::channel();
    let (entered, waiting) = mpsc::channel();
    let (returned, result) = mpsc::channel();
    let caller = thread::spawn(move || {
        let mut config = NativeConfig::new(Team::Radiant, vec![BotSeat { slot: bota_proto::SlotId(0), hero: bota_proto::HeroId::SHADOW_FIEND }], Keys { hash:[17,19], random:23 });
        config.startup_timeout = Duration::from_millis(1);
        let outcome = NativeTeam::start(config, ModuleBank::new(Limits::DEFAULT), move || { entered.send(()).expect("entered"); blocked.recv().expect("release"); });
        returned.send(matches!(outcome, Err(NativeError::Deadline))).expect("result");
    });
    waiting.recv_timeout(HANG_GUARD).expect("setup blocked");
    let completed = result.recv_timeout(HANG_GUARD).ok();
    release.send(()).expect("release");
    caller.join().expect("caller exits");
    assert_eq!(completed, Some(true), "startup timeout synchronously joined setup");
}

#[test]
fn native_limits_cannot_disable_fuel_enforcement() {
    let mut config=NativeConfig::new(Team::Radiant,vec![BotSeat{slot:bota_proto::SlotId(0),hero:bota_proto::HeroId::SHADOW_FIEND}],Keys{hash:[17,19],random:23});
    config.limits.fuel_per_call=u64::MAX;
    assert!(matches!(NativeTeam::new(config,ModuleBank::new(Limits::DEFAULT)),Err(NativeError::Setup(_))));
}
