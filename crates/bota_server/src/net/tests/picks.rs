//! A seat picks only an allowed hero, and its pick is final once it was
//! ready with it.

use super::*;
use bota_proto::SeatKey;

use crate::lobby::ReservedSeat;

/// Slot 0 of the lobby states a client is told until `done` holds for one.
fn first_seat_until(
    client: &mut TestClient,
    mut done: impl FnMut(&bota_proto::LobbySlot) -> bool,
) -> Vec<bota_proto::LobbySlot> {
    let mut seen = Vec::new();
    loop {
        let slot = client.recv_until(|msg| match msg {
            ServerMsg::LobbyState { mut slots, .. } => Some(slots.remove(0)),
            _ => None,
        });
        let last = done(&slot);
        seen.push(slot);
        if last {
            return seen;
        }
    }
}

#[test]
fn a_hero_not_allowed_is_never_picked() {
    let mut opts = realtime(30);
    opts.heroes = Some(vec![HeroId::SHADOW_FIEND]);
    let addr = start_server(opts);
    let mut client = TestClient::connect(addr);
    client.hello(Role::Bot, None);
    for msg in [
        ClientMsg::PickHero {
            hero: HeroId::PUDGE,
        },
        ClientMsg::SetReady(false),
        ClientMsg::PickHero {
            hero: HeroId::SHADOW_FIEND,
        },
    ] {
        client.send(&msg);
    }
    let seen = first_seat_until(&mut client, |slot| slot.hero.is_some());
    let heroes: Vec<Option<HeroId>> = seen.iter().map(|slot| slot.hero).collect();
    assert!(
        seen.len() >= 3,
        "the refused pick changed nothing: {heroes:?}"
    );
    assert_eq!(heroes.last(), Some(&Some(HeroId::SHADOW_FIEND)));
}

#[test]
fn a_pick_is_final_once_the_seat_was_ready_with_it() {
    let addr = start_server(realtime(31));
    let mut client = TestClient::connect(addr);
    client.hello(Role::Bot, None);
    let switch = ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    };
    for msg in [
        ClientMsg::PickHero {
            hero: HeroId::PUDGE,
        },
        ClientMsg::SetReady(true),
        switch.clone(),
        ClientMsg::SetReady(false),
        switch.clone(),
        ClientMsg::SetReady(true),
    ] {
        client.send(&msg);
    }
    let mut readiness = 0;
    let seen = first_seat_until(&mut client, |slot| {
        readiness += usize::from(slot.hero.is_some() && slot.ready);
        readiness == 2
    });
    assert!(
        seen.iter()
            .all(|slot| slot.hero != Some(HeroId::SHADOW_FIEND)),
        "{seen:?}"
    );
}

#[test]
fn a_seat_without_a_pick_is_given_an_allowed_hero() {
    // Whatever the seed would draw from every hero, a single allowed one is
    // what each seat gets.
    for hero in [HeroId::PUDGE, HeroId::SHADOW_FIEND] {
        let mut opts = realtime(32);
        let keys = [SeatKey([1; 32]), SeatKey([2; 32])];
        opts.reserved = keys
            .iter()
            .zip([Team::Radiant, Team::Dire])
            .map(|(key, team)| ReservedSeat {
                kind: None,
                key: *key,
                name: "absent".to_owned(),
                team,
            })
            .collect();
        opts.referee_key = Some(SeatKey([9; 32]));
        opts.start_after = Some(Duration::from_millis(300));
        opts.heroes = Some(vec![hero]);
        let addr = start_server(opts);
        let mut referee = TestClient::connect(addr);
        referee.hello(Role::Referee, Some(SeatKey([9; 32])));
        let info = referee.match_start();
        assert!(
            info.picks.iter().all(|pick| pick.hero == hero),
            "{:?}",
            info.picks
        );
    }
}
