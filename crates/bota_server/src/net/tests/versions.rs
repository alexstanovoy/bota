//! Version identities through the real game connection, not the display name.
use super::*;

#[test]
fn a_direct_game_accepts_a_structured_bot_version_hello() {
    let addr = start_server(realtime(3401));
    let mut client = TestClient::connect(addr);
    client.send(&ClientMsg::hello_with_version(
        Role::Bot,
        "engine",
        None,
        bota_core::WIRE_FINGERPRINT,
        Some("v1.0.0 (buster)".into()),
    ));
    client.welcome();
    let identity = client.recv_until(|message| match message {
        ServerMsg::SeatIdentity(identity) => Some(identity),
        _ => None,
    });
    assert_eq!(identity.name, "engine");
    assert_eq!(identity.version.as_deref(), Some("v1.0.0 (buster)"));
    let slots = client.recv_until(|message| match message {
        ServerMsg::LobbyState { slots, .. } => Some(slots),
        _ => None,
    });
    assert_eq!(slots[0].name, "engine");
    assert_eq!(slots[0].role, Some(Role::Bot));
}

#[test]
fn a_service_reservation_overrides_any_claimed_version() {
    use crate::lobby::{ReservedSeat, VersionSource};
    let key = bota_proto::SeatKey([79; 32]);
    let mut opts = realtime(3402);
    opts.players = 1;
    opts.reserved = vec![ReservedSeat {
        key,
        name: "engine".into(),
        team: Team::Radiant,
        kind: Some(Role::Bot),
        version: VersionSource::Fixed(Some("v1 (official)".into())),
    }];
    let mut client = TestClient::connect(start_server(opts));
    client.send(&ClientMsg::hello_with_version(
        Role::Bot,
        "spoofed",
        Some(key),
        bota_core::WIRE_FINGERPRINT,
        Some("bad\nclaim".into()),
    ));
    client.welcome();
    let identity = client.recv_until(|message| match message {
        ServerMsg::SeatIdentity(identity) => Some(identity),
        _ => None,
    });
    assert_eq!(identity.name, "engine");
    assert_eq!(identity.version.as_deref(), Some("v1 (official)"));
}

#[test]
fn direct_versions_refuse_invalid_values_with_the_exact_reason() {
    for version in [
        String::new(),
        "x".repeat(33),
        " v1".into(),
        "v1 ".into(),
        "v1\nline".into(),
        "v1\u{200b}".into(),
    ] {
        let mut client = TestClient::connect(start_server(realtime(3403)));
        client.send(&ClientMsg::hello_with_version(
            Role::Bot,
            "engine",
            None,
            bota_core::WIRE_FINGERPRINT,
            Some(version),
        ));
        assert_eq!(
            client.recv(),
            ServerMsg::Refused {
                reason: RejectReason::BadVersion
            }
        );
    }
}

#[test]
fn a_direct_human_cannot_state_a_bot_version() {
    let mut client = TestClient::connect(start_server(realtime(3404)));
    client.send(&ClientMsg::hello_with_version(
        Role::Player,
        "person",
        None,
        bota_core::WIRE_FINGERPRINT,
        Some("v1".into()),
    ));
    assert_eq!(
        client.recv(),
        ServerMsg::Refused {
            reason: RejectReason::VersionNotAllowed
        }
    );
}

#[test]
fn a_seat_version_reaches_late_spectators_and_the_recorded_message() {
    // In the temporary directory, like the other net tests: a checkout built with CARGO_TARGET_DIR has no `target/`.
    let path = std::env::temp_dir().join(format!("bota-seat-version-{}.brp", std::process::id()));
    let mut opts = realtime(3405);
    // Lockstep: the match moves only when its seats acknowledge a tick, which this bot never does,
    // so no clock, no stream of ticks to a client that is not reading and no catching up of a
    // starved machine can come between the late spectator and what the test waits for.
    opts.mode = bota_proto::TickMode::Lockstep;
    opts.players = 1;
    opts.map = bota_core::MAP2_ID;
    opts.spectators = Spectators::Open;
    opts.replay = Some(path.clone());
    let server = start_ending_server(opts);
    let mut bot = TestClient::connect(server.addr);
    bot.send(&ClientMsg::hello_with_version(
        Role::Bot,
        "engine",
        None,
        bota_core::WIRE_FINGERPRINT,
        Some("v1 (recorded)".into()),
    ));
    bot.welcome();
    bot.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    bot.send(&ClientMsg::SetReady(true));
    bot.recv_until(snapshot);
    let mut watcher = TestClient::connect(server.addr);
    watcher.send(&ClientMsg::hello(
        Role::Spectator,
        "watcher",
        None,
        bota_core::WIRE_FINGERPRINT,
    ));
    watcher.welcome();
    let identity = watcher.recv_until(|message| match message {
        ServerMsg::SeatIdentity(identity) => Some(identity),
        _ => None,
    });
    assert_eq!(identity.version.as_deref(), Some("v1 (recorded)"));
    bot.send(&ClientMsg::LeaveGame);
    drop((bot, watcher));
    server.end();
    let bytes = read_replay(&path).expect("completed recording");
    let mut reader = FrameReader::new();
    reader.push(&bytes);
    let mut found = None;
    while let Some(record) = reader.next_message::<ReplayRecord>().expect("record frame") {
        if let ReplayRecord::Msg(ServerMsg::SeatIdentity(identity)) = record {
            found = Some(identity);
        }
    }
    assert_eq!(found, Some(identity));
    std::fs::remove_file(path).expect("remove private recording");
}
