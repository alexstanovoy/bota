//! A seat picks only an allowed hero, and its pick is final once it was
//! ready with it.

use super::*;
use bota_proto::{MapId, SeatKey};

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
    // One seat that may not share: the pool of one is then left to the seat to pick.
    opts.players = 1;
    opts.same_heroes = false;
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

/// The heroes two absent seats are given once the lobby runs out of time,
/// on a map, from a seed, with Pudge and Shadow Fiend allowed.
fn heroes_given(map: MapId, seed: u64, allowed: Vec<HeroId>) -> Vec<HeroId> {
    let mut opts = realtime(seed);
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
    opts.map = map;
    opts.heroes = Some(allowed);
    let addr = start_server(opts);
    let mut referee = TestClient::connect(addr);
    referee.hello(Role::Referee, Some(SeatKey([9; 32])));
    referee
        .match_start()
        .picks
        .iter()
        .map(|pick| pick.hero)
        .collect()
}

#[test]
fn a_seat_without_a_pick_is_given_an_allowed_hero() {
    // Whatever the seed would draw from every hero, a single allowed one is
    // what each seat gets.
    for hero in [HeroId::PUDGE, HeroId::SHADOW_FIEND] {
        assert_eq!(heroes_given(MapId(0), 32, vec![hero]), [hero, hero]);
    }
}

/// The mid-only map's 1v1 is a Shadow Fiend mirror: a seat that picks none
/// plays Shadow Fiend there whatever else is allowed, where the Dota map,
/// which names no default hero, draws one of the allowed.
#[test]
fn a_seat_without_a_pick_plays_the_maps_default_hero() {
    let both = || vec![HeroId::PUDGE, HeroId::SHADOW_FIEND];
    let fiend = [HeroId::SHADOW_FIEND; 2];
    for seed in 33..37 {
        assert_eq!(heroes_given(MapId(2), seed, both()), fiend, "seed {seed}");
    }
    let drawn: Vec<HeroId> = (33..37)
        .flat_map(|seed| heroes_given(MapId(0), seed, both()))
        .collect();
    assert!(
        drawn.contains(&HeroId::PUDGE),
        "drawn on the Dota map: {drawn:?}"
    );
    // Not allowed, it is not given.
    assert_eq!(
        heroes_given(MapId(2), 33, vec![HeroId::PUDGE]),
        [HeroId::PUDGE; 2]
    );
}

/// A pool of one hero that two seats may share is a forced pick: the server tells it, and gives
/// it to each seat as it is taken, ready; nobody chooses, and the match starts once both are held.
#[test]
fn a_pool_of_one_is_given_to_every_seat_ready() {
    let mut opts = realtime(32);
    opts.map = MapId(2);
    opts.heroes = Some(vec![HeroId::SHADOW_FIEND]);
    opts.same_heroes = true;
    let addr = start_server(opts);
    let mut seats = [TestClient::connect(addr), TestClient::connect(addr)];
    for seat in &mut seats {
        seat.hello(Role::Bot, None);
        seat.welcome();
    }
    // Neither says a word more: the lobby tells the pool and both seats picked and ready.
    for seat in &mut seats {
        let (heroes, slots) = seat.recv_until(|msg| match msg {
            ServerMsg::LobbyState { heroes, slots, .. }
                if slots.iter().all(|s| s.hero.is_some() && s.ready) =>
            {
                Some((heroes, slots))
            }
            _ => None,
        });
        assert_eq!(heroes, [HeroId::SHADOW_FIEND]);
        assert!(slots.iter().all(|s| s.hero == Some(HeroId::SHADOW_FIEND)));
    }
    let view = seats[0].recv_until(|msg| snapshot(msg).filter(|view| view.tick > 1));
    assert_eq!(view.players.len(), 2);
}
