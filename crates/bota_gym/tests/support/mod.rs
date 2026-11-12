//! Matches, bots and digests the gym's end-to-end tests share.

#![allow(
    dead_code,
    reason = "each test binary uses its own part of the support"
)]

use std::hash::{DefaultHasher, Hash, Hasher};

use bota_client_core::{ClientSession, IssuedOrder};
use bota_core::{MAP2_ID, map_of};
use bota_gym::{GameId, MAX_SEATS, MatchSpec, PlayerEnv, PlayerEnvConfig, SeatOrders, derive_seed};
use bota_proto::{HeroId, Order, Pick, SlotId, Target, Team, Vec2};

pub const MATCH_DOMAIN: u64 = u64::from_le_bytes(*b"testgame");
const BOT_DOMAIN: u64 = u64::from_le_bytes(*b"test_bot");
pub const RUN_SEED: u64 = 20_261_113;

/// Pudge against Shadow Fiend on the mid-only map.
pub fn picks() -> Vec<Pick> {
    let pick = |slot, team, hero| Pick {
        slot: SlotId(slot),
        team,
        hero,
    };
    vec![
        pick(0, Team::Radiant, HeroId::PUDGE),
        pick(1, Team::Dire, HeroId::SHADOW_FIEND),
    ]
}

/// The tick `seconds` of game time after the horn.
pub fn after_horn(seconds: u32) -> u32 {
    map_of(MAP2_ID).pregame_ticks + seconds * 30
}

/// A duel of `seed`, cut `seconds` after the horn.
pub fn duel(seed: u64, seconds: u32) -> MatchSpec {
    MatchSpec::new(MAP2_ID, picks(), seed, after_horn(seconds))
}

/// Duels cut `seconds` after the horn, seeded by game id; decisions every three ticks.
pub fn config(seconds: u32) -> PlayerEnvConfig<()> {
    PlayerEnvConfig {
        decision_ticks: 3,
        spec: Box::new(move |game: GameId, _: &()| {
            duel(
                game.seed(RUN_SEED, MATCH_DOMAIN)
                    .expect("a slot below MAX_SLOTS"),
                seconds,
            )
        }),
    }
}

/// A bot that reads only its own seat's client: on a seeded tenth of decisions it attack-moves
/// to a seeded spot between the fountains. Its draws depend on the game, seat and tick alone.
pub fn random_orders<P, A: bota_gym::Access>(env: &PlayerEnv<P, A>, game: GameId) -> SeatOrders {
    let [radiant, dire] = map_of(MAP2_ID).fountains;
    let mut orders: SeatOrders = [None; MAX_SEATS];
    for (seat, order) in orders.iter_mut().enumerate().take(env.seat_count()) {
        let session = env.seat(seat);
        let Some(view) = session.current() else {
            continue;
        };
        if session.own_hero().is_none() {
            continue;
        }
        let stream = game.key().expect("a key") * 16 + seat as u64;
        let draw = derive_seed(
            derive_seed(RUN_SEED, stream, BOT_DOMAIN),
            view.tick.into(),
            0,
        );
        if draw.is_multiple_of(10) {
            let along = (draw >> 8) % 1001;
            *order = Some(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(between(radiant, dire, along as i32)),
            }));
        }
    }
    orders
}

/// Every seat attack-moves to the middle of the map, again every ten seconds (a hero that fell
/// walks back): enough to farm and fight.
pub fn to_the_middle<P, A: bota_gym::Access>(env: &PlayerEnv<P, A>) -> SeatOrders {
    let [radiant, dire] = map_of(MAP2_ID).fountains;
    let mut orders: SeatOrders = [None; MAX_SEATS];
    for (seat, order) in orders.iter_mut().enumerate().take(env.seat_count()) {
        let session = env.seat(seat);
        let due = session.current().is_some_and(|view| view.tick % 300 < 3);
        if due && session.own_hero().is_some() {
            *order = Some(IssuedOrder::hero(Order::Attack {
                target: Target::Pos(between(radiant, dire, 500)),
            }));
        }
    }
    orders
}

pub fn between(from: Vec2, to: Vec2, along: i32) -> Vec2 {
    let at = |a: i32, b: i32| a + (b - a) * along / 1000;
    Vec2::from_ints(
        at(from.x.to_int(), to.x.to_int()),
        at(from.y.to_int(), to.y.to_int()),
    )
}

/// What a seat's client knows now, as one number.
pub fn digest(session: &ClientSession) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", session.current()).hash(&mut hasher);
    hasher.finish()
}
