//! A permanent departure revokes seat-key admission for both reservation modes.
use super::*;
use crate::lobby::ReservedSeat;
use bota_proto::SeatKey;

#[test]
fn a_departed_claim_key_cannot_reclaim_or_open_a_fogged_spectator_view() {
    assert_revoked(false);
}

#[test]
fn a_departed_reserved_key_cannot_reclaim_or_open_a_fogged_spectator_view() {
    assert_revoked(true);
}

fn running_three(
    reserved: bool,
    spectators: Spectators,
) -> (RunningServer, Vec<TestClient>, [SeatKey; 3]) {
    let keys = [SeatKey([1; 32]), SeatKey([2; 32]), SeatKey([3; 32])];
    let mut opts = realtime(5);
    opts.players = 3;
    opts.spectators = spectators;
    opts.mode = bota_proto::TickMode::Lockstep;
    opts.empty_after = Some(Duration::from_secs(1));
    if reserved {
        opts.reserved = keys
            .iter()
            .enumerate()
            .map(|(i, key)| ReservedSeat {
                key: *key,
                name: format!("seat{i}"),
                team: if i == 1 { Team::Dire } else { Team::Radiant },
                kind: None,
            })
            .collect();
    }
    let server = start_ending_server(opts);
    let mut clients = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        let mut client = TestClient::connect(server.addr);
        client.hello(Role::Bot, Some(*key));
        assert_eq!(client.welcome(), Some(SlotId(i as u8)));
        client.send(&ClientMsg::PickHero {
            hero: HeroId::SHADOW_FIEND,
        });
        client.send(&ClientMsg::SetReady(true));
        clients.push(client);
    }
    for client in &mut clients {
        client.match_start();
        client.collect_through_tick(1);
    }
    (server, clients, keys)
}

fn assert_revoked(reserved: bool) {
    let (server, mut clients, keys) = running_three(reserved, Spectators::Off);
    clients[0].send(&ClientMsg::LeaveGame);
    for client in &mut clients {
        client.send(&ClientMsg::Ack { tick: 1 });
    }
    // Tick two requires either the leaver's post-leave Ack (old behavior) or its
    // permanent departure (fixed behavior), so LeaveGame has been handled in both.
    clients[1].collect_through_tick(2);
    let mut watcher = TestClient::connect(server.addr);
    watcher.hello(Role::Spectator, Some(keys[0]));
    assert_eq!(
        watcher.refusal(),
        RejectReason::NotAdmitted,
        "reserved={reserved}"
    );
    drop(watcher);
    drop(clients.remove(0));
    let mut reclaim = TestClient::connect(server.addr);
    reclaim.hello(Role::Bot, Some(keys[0]));
    assert_eq!(
        reclaim.refusal(),
        RejectReason::NotAdmitted,
        "reserved={reserved}"
    );
    drop(reclaim);
    drop(clients);
    let end = server
        .ended
        .recv_timeout(HANG_GUARD)
        .expect("server ended")
        .expect("ran");
    let MatchEnd::Abandoned { leaves } = end else {
        panic!("remaining seats only disconnected: {end:?}")
    };
    assert_eq!(
        leaves,
        vec![bota_proto::MatchLeave {
            slot: SlotId(0),
            tick: 1
        }]
    );
}

#[test]
fn an_open_spectator_can_choose_a_departed_seats_public_team_view() {
    let (server, mut clients, _) = running_three(false, Spectators::Open);
    clients[0].send(&ClientMsg::LeaveGame);
    for client in &mut clients {
        client.send(&ClientMsg::Ack { tick: 1 });
    }
    for client in &mut clients[1..] {
        client.collect_through_tick(2);
    }
    let mut watcher = TestClient::connect(server.addr);
    watcher.hello(Role::Spectator, None);
    assert_eq!(watcher.welcome(), None);
    watcher.match_start();
    watcher.send(&ClientMsg::ViewAs {
        seat: Some(SlotId(0)),
    });
    let started = Instant::now();
    let mut tick = 2;
    loop {
        assert!(
            started.elapsed() < HANG_GUARD,
            "public ViewAs was never applied"
        );
        for client in &mut clients[1..] {
            client.send(&ClientMsg::Ack { tick });
        }
        tick += 1;
        for client in &mut clients[1..] {
            client.collect_through_tick(tick);
        }
        let view = watcher.recv_until(|msg| snapshot(msg).filter(|view| view.tick >= tick));
        if view.viewer == Some(Team::Radiant) {
            break;
        }
    }
    drop(watcher);
    drop(clients);
    let end = server
        .ended
        .recv_timeout(HANG_GUARD)
        .expect("server ended")
        .expect("ran");
    assert!(matches!(end, MatchEnd::Abandoned { .. }));
}
