//! The clock before the match: the selection time and its penalty, the
//! countdown the clients are told, the tactical phase, and the pick rules.

use super::*;
use bota_proto::{LobbySlot, PickPhase, SeatKey};

use crate::lobby::ReservedSeat;

fn lobby(msg: ServerMsg) -> Option<(Vec<LobbySlot>, PickPhase)> {
    match msg {
        ServerMsg::LobbyState { slots, pick, .. } => Some((slots, pick)),
        _ => None,
    }
}

/// A client that says Hello with `role` and is welcomed to its seat, so the
/// seats go in the order the clients came.
fn greeted(addr: std::net::SocketAddr, role: Role) -> TestClient {
    let mut client = TestClient::connect(addr);
    client.send(&ClientMsg::hello(role, "c", None));
    client.recv_until(|msg| matches!(msg, ServerMsg::Welcome { .. }).then_some(()));
    client
}

fn match_start_picks_of(client: &mut TestClient) -> Vec<(SlotId, HeroId)> {
    client.recv_until(|msg| match msg {
        ServerMsg::MatchStart { info } => {
            Some(info.picks.iter().map(|p| (p.slot, p.hero)).collect())
        }
        _ => None,
    })
}

fn quick(seed: u64) -> ServerOpts {
    let mut opts = realtime(seed);
    opts.pick_time = Duration::from_millis(200);
    opts.pick_penalty_time = Duration::from_millis(300);
    opts
}

#[test]
fn a_timeout_gives_each_seat_the_same_hero_whenever_the_seed_is_the_same() {
    let mut runs = Vec::new();
    for _ in 0..2 {
        let addr = start_server(quick(77));
        let mut first = greeted(addr, Role::Bot);
        let _second = greeted(addr, Role::Bot);
        runs.push(match_start_picks_of(&mut first));
    }
    assert_eq!(runs[0], runs[1]);
    assert_eq!(runs[0].len(), 2);
}

#[test]
fn a_timeout_draws_among_the_allowed_heroes_and_never_a_hero_taken_when_duplicates_are_off() {
    let mut opts = quick(78);
    opts.same_heroes = false;
    opts.heroes = Some(vec![HeroId::SHADOW_FIEND, HeroId::PUDGE]);
    let addr = start_server(opts);
    let mut first = greeted(addr, Role::Bot);
    first.send(&ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    });
    first.recv_until(|msg| lobby(msg).filter(|(slots, _)| slots[0].hero.is_some()));
    let mut second = greeted(addr, Role::Bot);
    let picks = match_start_picks_of(&mut second);
    assert_eq!(
        picks,
        vec![
            (SlotId(0), HeroId::SHADOW_FIEND),
            (SlotId(1), HeroId::PUDGE)
        ]
    );
}

#[test]
fn a_seat_picking_late_starts_with_less_gold() {
    let mut opts = realtime(79);
    opts.pick_time = Duration::from_secs(1);
    opts.pick_penalty_time = Duration::from_secs(10);
    let addr = start_server(opts);
    let mut punctual = greeted(addr, Role::Bot);
    let pick = ClientMsg::PickHero {
        hero: HeroId::SHADOW_FIEND,
    };
    punctual.send(&pick);
    punctual.send(&ClientMsg::SetReady(true));
    punctual.recv_until(|msg| lobby(msg).filter(|(slots, _)| slots[0].hero.is_some()));
    // The clock starts with the second seat; this one picks after it ran out.
    let mut tardy = greeted(addr, Role::Bot);
    thread::sleep(Duration::from_millis(2_300));
    tardy.send(&pick);
    tardy.send(&ClientMsg::SetReady(true));
    let gold = |client: &mut TestClient| {
        client
            .recv_until(snapshot)
            .players
            .iter()
            .find_map(|p| p.gold)
    };
    assert_eq!(gold(&mut punctual), Some(bota_core::rules::STARTING_GOLD));
    let lost = bota_core::rules::STARTING_GOLD - gold(&mut tardy).expect("own gold");
    assert!((2..=8).contains(&lost) && lost % 2 == 0, "lost {lost}");
}

#[test]
fn the_countdown_reaches_clients_and_a_reconnect_sees_the_time_left() {
    // A clock no stall of a loaded machine runs out: the test waits for a
    // span to pass, never for the clock to end.
    let pick_time = Duration::from_secs(120);
    let mut opts = realtime(80);
    opts.pick_time = pick_time;
    let addr = start_server(opts);
    let mut first = greeted(addr, Role::Player);
    let _second = greeted(addr, Role::Player);
    let left = |client: &mut TestClient| {
        client.recv_until(|msg| match lobby(msg) {
            Some((_, PickPhase::Selecting { left_ms })) => Some(left_ms),
            _ => None,
        })
    };
    let told = left(&mut first);
    let heard = Instant::now();
    assert!(u128::from(told) <= pick_time.as_millis(), "{told}");
    assert!(
        told >= 60_000,
        "a minute went by before the first word: {told}"
    );
    drop(first);
    let span = Duration::from_millis(1_200);
    while let Some(rest) = span.checked_sub(heard.elapsed()) {
        thread::sleep(rest);
    }
    let mut back = greeted(addr, Role::Player);
    let later = left(&mut back);
    assert!(
        later <= told - 1_000,
        "the time kept running: {told} then {later}"
    );
    assert!(later > 0);
}

#[test]
fn two_bots_skip_the_tactical_phase() {
    let mut opts = realtime(81);
    opts.tactical_time = Duration::from_secs(60);
    let addr = start_server(opts);
    let started = Instant::now();
    let (mut first, _) = join_as_bot(addr, "a");
    let (_second, _) = join_as_bot(addr, "b");
    first.recv_until(match_start_picks);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_human_seat_waits_out_the_tactical_phase() {
    let mut opts = realtime(82);
    opts.tactical_time = Duration::from_millis(1_500);
    let addr = start_server(opts);
    let mut human = greeted(addr, Role::Player);
    let (_bot, _) = join_as_bot(addr, "b");
    for msg in [
        ClientMsg::PickHero {
            hero: HeroId::SHADOW_FIEND,
        },
        ClientMsg::SetReady(true),
    ] {
        human.send(&msg);
    }
    let mut tactical_since = None;
    human.recv_until(|msg| {
        match &msg {
            ServerMsg::LobbyState {
                pick: PickPhase::Tactical { left_ms },
                ..
            } => {
                assert!(*left_ms <= 1_500);
                tactical_since.get_or_insert_with(Instant::now);
            }
            ServerMsg::MatchStart { .. } => return Some(()),
            _ => {}
        }
        None
    });
    let waited = tactical_since
        .expect("the tactical phase was announced")
        .elapsed();
    assert!(waited >= Duration::from_millis(1_000), "{waited:?}");
}

#[test]
fn a_taken_hero_is_refused_unless_the_same_hero_is_allowed() {
    for same in [true, false] {
        let mut opts = realtime(83);
        opts.same_heroes = same;
        let addr = start_server(opts);
        let mut first = greeted(addr, Role::Bot);
        let mut second = greeted(addr, Role::Bot);
        let pick = ClientMsg::PickHero {
            hero: HeroId::SHADOW_FIEND,
        };
        first.send(&pick);
        first.recv_until(|msg| lobby(msg).filter(|(slots, _)| slots[0].hero.is_some()));
        second.send(&pick);
        second.send(&ClientMsg::PickHero {
            hero: HeroId::PUDGE,
        });
        let heroes = second.recv_until(|msg| {
            lobby(msg)
                .map(|(slots, _)| slots[1].hero)
                .filter(Option::is_some)
        });
        let expected = if same {
            HeroId::SHADOW_FIEND
        } else {
            HeroId::PUDGE
        };
        assert_eq!(heroes, Some(expected), "same heroes allowed: {same}");
    }
}

#[test]
fn a_reserved_kind_refuses_the_other_kind_of_player() {
    let mut opts = realtime(84);
    let key = SeatKey([3; 32]);
    opts.reserved = [Team::Radiant, Team::Dire]
        .iter()
        .enumerate()
        .map(|(at, team)| ReservedSeat {
            key: SeatKey([3 + at as u8; 32]),
            name: "seat".to_owned(),
            team: *team,
            kind: Some(Role::Bot),
        })
        .collect();
    let addr = start_server(opts);
    let mut human = TestClient::connect(addr);
    human.send(&ClientMsg::hello(Role::Player, "h", Some(key)));
    assert_eq!(
        human.recv(),
        ServerMsg::Refused {
            reason: RejectReason::NotAdmitted
        }
    );
    let mut bot = TestClient::connect(addr);
    bot.send(&ClientMsg::hello(Role::Bot, "b", Some(key)));
    assert!(matches!(bot.recv(), ServerMsg::Welcome { .. }));
}
