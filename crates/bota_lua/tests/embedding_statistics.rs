#![cfg(all(feature = "native", target_os = "linux"))]
mod support;
use bota_client_core::ReplayPlayer;
use bota_lua::{BotSeat, Keys, LuaTeam, ModuleBank, SCRIPT_LIMITS, native::refeed};
use bota_proto::{HeroId, ReplayHeader, ReplayRecord, SlotId, Team};
use std::io::Cursor;
fn keys() -> Keys {
    Keys {
        hash: [17, 19],
        random: 23,
    }
}
fn modules() -> ModuleBank {
    let mut bank = ModuleBank::new(SCRIPT_LIMITS);
    bank.insert(
        "bots/bot_nevermore",
        br#"function Think()
      local n=0; for k,v in pairs({a=1,b=7,c=31}) do n=n*33+v end
      local x=GetBot():GetLocation().x
      GetBot():Action_MoveToLocation(Vector(math.random(0,100)+n,0,0))
    end"#,
    )
    .unwrap();
    bank
}
fn frame(bytes: &mut Vec<u8>, record: ReplayRecord) {
    bota_proto::encode_frame(&record, bytes).unwrap();
}
fn play(measured: bool) -> (Vec<u8>, Vec<(u32, Vec<bota_client_core::IssuedOrder>)>) {
    let (mut local, opening) = support::opening(&[Team::Radiant, Team::Dire]);
    let mut bytes = Vec::new();
    frame(
        &mut bytes,
        ReplayRecord::Header(ReplayHeader::new(bota_core::WIRE_FINGERPRINT)),
    );
    for msg in &opening[0] {
        frame(&mut bytes, ReplayRecord::Msg(msg.clone()));
    }
    let mut sessions = opening
        .into_iter()
        .enumerate()
        .map(|(index, stream)| {
            let bota_proto::ServerMsg::MatchStart { info } = &stream[0] else {
                panic!("start")
            };
            let mut session = bota_client_core::ClientSession::seat(
                bota_client_core::MatchStatic::new(info).unwrap(),
                SlotId(index as u8),
                Some(HeroId::SHADOW_FIEND),
                Default::default(),
            )
            .unwrap();
            support::fold(&mut session, stream.into_iter().skip(1));
            session
        })
        .collect::<Vec<_>>();
    let limits = SCRIPT_LIMITS
        .with_fuel_per_call(125_000)
        .with_fuel_per_tick(600_000);
    let mut lua = LuaTeam::new_measured(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        limits,
        keys(),
        measured,
    )
    .unwrap();
    lua.set_modules(modules()).unwrap();
    lua.load_pack().unwrap();
    let mut orders = Vec::new();
    for _ in 0..16 {
        let turn = lua.think(&sessions[0]).unwrap();
        assert!(turn.failures.is_empty());
        let tick = sessions[0].current().unwrap().tick;
        orders.push((tick, turn.bots[0].orders.clone()));
        let outgoing = turn.bots[0]
            .orders
            .iter()
            .map(|&order| sessions[0].issue(order).unwrap())
            .collect::<Vec<_>>();
        let streams = local.step(&[&outgoing, &[]]).unwrap();
        for msg in &streams[0] {
            frame(&mut bytes, ReplayRecord::Msg(msg.clone()));
        }
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
    // The final recorded tick is also a complete input to the offline host.
    let turn = lua.think(&sessions[0]).unwrap();
    orders.push((
        sessions[0].current().unwrap().tick,
        turn.bots[0].orders.clone(),
    ));
    (bytes, orders)
}
#[test]
fn explicit_keys_reproduce_live_orders_and_two_offline_refeeds() {
    let (bytes, live) = play(false);
    let (_, measured) = play(true);
    assert_eq!(live, measured);
    let run = || {
        let mut orders = Vec::new();
        let outcome = refeed(
            ReplayPlayer::from_reader(Cursor::new(bytes.clone())),
            SlotId(0),
            modules(),
            keys(),
            bota_lua::native::HeroSelection::Fixed,
            |tick, turn| orders.push((tick, turn.bots[0].orders.clone())),
        )
        .unwrap();
        (orders, outcome)
    };
    let (a, first) = run();
    let (b, second) = run();
    assert_eq!(live, a);
    assert_eq!(a, b);
    assert_eq!(first.statistics.turns, 17);
    assert_eq!(first.statistics.script_failures, 0);
    assert_eq!(
        serde_json::to_value(&first.statistics.fuel).unwrap(),
        serde_json::to_value(&second.statistics.fuel).unwrap()
    );
    let calls = |result: &bota_lua::native::RefeedOutcome| {
        result
            .statistics
            .api
            .iter()
            .map(|entry| (entry.name, entry.calls))
            .collect::<Vec<_>>()
    };
    assert_eq!(calls(&first), calls(&second));
    assert!(
        !first
            .statistics
            .api
            .iter()
            .any(|entry| entry.name.starts_with("__"))
    );
    assert!(
        first
            .statistics
            .api
            .iter()
            .any(|entry| entry.name == "other" && entry.calls >= 17)
    );
    assert!(
        first
            .statistics
            .api
            .iter()
            .any(|entry| entry.name == "Action_MoveToLocation" && entry.calls == 17)
    );
}
#[test]
fn hostile_summary_strings_are_capped_escaped_and_atomically_replaced() {
    let path = std::env::temp_dir().join(format!("embedding-summary-{}.json", std::process::id()));
    std::fs::write(&path, b"previous").unwrap();
    let mut stats = bota_lua::statistics::RunStatistics::default();
    stats.failure(&format!("\"\\\n\0{}", "x".repeat(8000)));
    for tick in 0..2000 {
        stats.discarded.record(tick);
    }
    stats
        .write_atomic(&path, keys(), serde_json::json!({"ended":true}))
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() < 10000);
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed["rng_seed"], 23);
    assert_eq!(
        parsed["statistics"]["discarded"]["ticks"]
            .as_array()
            .unwrap()
            .len(),
        256
    );
    assert_eq!(
        parsed["statistics"]["first_error"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        1024
    );
    std::fs::remove_file(path).unwrap();
}
#[test]
fn refeed_rejects_corrupt_frames_without_a_new_decoder() {
    assert!(
        refeed(
            ReplayPlayer::from_reader(Cursor::new(vec![255; 32])),
            SlotId(0),
            modules(),
            keys(),
            bota_lua::native::HeroSelection::Fixed,
            |_, _| {}
        )
        .is_err()
    );
}

#[test]
fn measured_native_turn_reports_exhaustion_on_its_exact_tick() {
    let mut bank = ModuleBank::new(SCRIPT_LIMITS);
    bank.insert(
        "bots/bot_nevermore",
        b"function Think() while true do end end",
    )
    .unwrap();
    let mut config = bota_lua::native::NativeConfig::new(
        Team::Radiant,
        vec![BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        keys(),
    );
    config.statistics = true;
    config.deterministic = true;
    config.hero_selection = bota_lua::native::HeroSelection::Fixed;
    let mut host = bota_lua::native::NativeTeam::new(config, bank).unwrap();
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let tick = sessions[0].current().unwrap().tick;
    let turn = host.think(&sessions[0]).unwrap();
    assert_eq!(turn.failures.len(), 1);
    let stats = host.statistics().unwrap();
    assert_eq!(stats.turns, 1);
    assert_eq!(stats.fuel_exhaustions.ticks, [tick]);
    assert_eq!(stats.script_failures, 1);
    assert_eq!(stats.discarded.ticks, [tick]);
    assert!(stats.first_error.is_some());
    assert_eq!(host.deadline_ticks(), 0);
}

#[test]
fn fogless_recording_is_rejected_instead_of_granting_hidden_state() {
    let (_, opening) = support::opening(&[Team::Radiant, Team::Dire]);
    let mut bytes = Vec::new();
    frame(
        &mut bytes,
        ReplayRecord::Header(ReplayHeader::new(bota_core::WIRE_FINGERPRINT)),
    );
    for mut message in opening[0].clone() {
        if let bota_proto::ServerMsg::Snapshot { view } = &mut message {
            view.viewer = None;
        }
        frame(&mut bytes, ReplayRecord::Msg(message));
    }
    let result = refeed(
        ReplayPlayer::from_reader(Cursor::new(bytes)),
        SlotId(0),
        modules(),
        keys(),
        bota_lua::native::HeroSelection::Fixed,
        |_, _| {},
    );
    assert!(result.err().unwrap().to_string().contains("fogless"));
}

#[test]
fn native_fallback_work_is_measured_as_host_work_without_changing_orders() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let run = |measured| {
        let mut lua = LuaTeam::new_measured(
            Team::Radiant,
            &[BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
            SCRIPT_LIMITS,
            keys(),
            measured,
        )
        .unwrap();
        lua.set_modules(ModuleBank::new(SCRIPT_LIMITS)).unwrap();
        lua.load_pack().unwrap();
        lua.think(&sessions[0]).unwrap()
    };
    let plain = run(false);
    let measured = run(true);
    assert!(measured.hook_activity.native > 0);
    assert_eq!(plain.bots[0].orders, measured.bots[0].orders);
    assert!(
        measured
            .statistics
            .unwrap()
            .api
            .iter()
            .any(|entry| entry.name == "other" && entry.calls > 0)
    );
}
