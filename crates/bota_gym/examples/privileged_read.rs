//! A policy that sees only what its client sees, beside a training signal that sees everything.
//!
//! ```text
//! cargo run --release -p bota_gym --example privileged_read
//! ```
//!
//! A critic may learn from the whole truth (the enemy's health behind the fog, say) while the
//! policy it trains must act on what a real client would know. In the gym the two are different
//! types: the policy below reads `PlayerEnv::seat`, a client session; the signal reads
//! `PlayerEnv::truth`, which only a `PlayerEnv<_, Privileged>` has. Swap `Privileged` for
//! `Player` and the signal no longer compiles, while the policy still does.

use bota_client_core::{ClientSession, IssuedOrder};
use bota_core::{MAP2_ID, map_of};
use bota_gym::{
    Episode, GameId, MAX_SEATS, MatchSpec, PlayerEnv, PlayerEnvConfig, Privileged, SeatOrders,
};
use bota_proto::{HeroId, Order, Pick, SlotId, Target, Team, Vec2};

fn main() {
    let config = PlayerEnvConfig {
        decision_ticks: 3,
        spec: Box::new(|game: GameId, _: &()| {
            let pick = |slot, team, hero| Pick {
                slot: SlotId(slot),
                team,
                hero,
            };
            let picks = vec![
                pick(0, Team::Radiant, HeroId::SHADOW_FIEND),
                pick(1, Team::Dire, HeroId::PUDGE),
            ];
            let two_minutes = map_of(MAP2_ID).pregame_ticks + 2 * 60 * 30;
            let seed = game
                .seed(7, u64::from_le_bytes(*b"privread"))
                .expect("a slot");
            MatchSpec::new(MAP2_ID, picks, seed, two_minutes)
        }),
    };
    let mut env = PlayerEnv::<(), Privileged>::start(&config, GameId { slot: 0, game: 0 }, ())
        .expect("a game");
    let (mut decisions, mut hidden, mut hidden_health) = (0u64, 0u64, 0i64);
    loop {
        let mut orders: SeatOrders = [None; MAX_SEATS];
        for (seat, order) in orders.iter_mut().enumerate().take(env.seat_count()) {
            *order = policy(env.seat(seat));
        }
        // The training signal: seat 0's view of the enemy hero's health, from the truth.
        let truth = env.truth().full_view();
        let enemy_health = truth
            .player(SlotId(1))
            .and_then(|player| player.unit)
            .and_then(|unit| truth.unit(unit))
            .map_or(0, |hero| hero.hp);
        let seen_by_policy = enemy_seen(env.seat(0));
        decisions += 1;
        if !seen_by_policy {
            hidden += 1;
            hidden_health += i64::from(enemy_health);
        }
        if let Some(finished) = env.step(&config, orders).expect("a step") {
            println!(
                "game over at tick {}: {decisions} decisions, the enemy hero hidden from seat 0's \
                 client in {hidden} of them; the signal knew its health all along (mean {} hp \
                 while hidden)",
                finished.tick,
                hidden_health / i64::try_from(hidden.max(1)).expect("a count")
            );
            return;
        }
    }
}

/// The policy: only what this seat's own client knows. Walk to the middle every ten seconds.
fn policy(session: &ClientSession) -> Option<IssuedOrder> {
    let tick = session.current()?.tick;
    session.own_hero()?;
    let [radiant, dire] = map_of(MAP2_ID).fountains;
    let middle = Vec2::from_ints(
        (radiant.x.to_int() + dire.x.to_int()) / 2,
        (radiant.y.to_int() + dire.y.to_int()) / 2,
    );
    (tick % 300 < 3).then_some(IssuedOrder::hero(Order::Attack {
        target: Target::Pos(middle),
    }))
}

/// Whether seat 0's client sees the enemy hero now.
fn enemy_seen(session: &ClientSession) -> bool {
    session.current().is_some_and(|view| {
        view.player(SlotId(1))
            .and_then(|player| player.unit)
            .and_then(|unit| view.unit(unit))
            .is_some()
    })
}
