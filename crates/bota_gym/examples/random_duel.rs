//! Random bots in concurrent mid-only duels: the smallest complete training-style loop.
//!
//! ```text
//! cargo run --release -p bota_gym --example random_duel
//! cargo run --release -p bota_gym --example random_duel -- --slots 32 --workers 8 --decisions 3000
//! ```
//!
//! Every slot plays Pudge against Shadow Fiend, game after game; a game is cut after five game
//! minutes. Every decision each seat's bot reads what its own client knows (its session) and
//! attack-moves to a random spot between the fountains, or keeps its order. Finished games are
//! printed with their scoreboard, then the throughput.

use std::time::Instant;

use bota_client_core::IssuedOrder;
use bota_core::{MAP2_ID, map_of};
use bota_gym::{
    GameId, MAX_SEATS, MatchSpec, PlayerEnv, PlayerEnvConfig, SeatOrders, SplitMix64, VecEnv,
    WorkerConfig, Workers,
};
use bota_proto::{HeroId, Order, Pick, SlotId, Target, Team, Vec2};

/// Any distinct constant: the seed domain of matches.
const MATCH_DOMAIN: u64 = u64::from_le_bytes(*b"example!");
const POLICY_DOMAIN: u64 = u64::from_le_bytes(*b"policy!!");

fn main() {
    let (slots, workers, decisions, seed) = arguments();
    let config = PlayerEnvConfig {
        // Ten decisions a second.
        decision_ticks: 3,
        spec: Box::new(move |game: GameId, _: &()| {
            let pick = |slot, team, hero| Pick {
                slot: SlotId(slot),
                team,
                hero,
            };
            let picks = vec![
                pick(0, Team::Radiant, HeroId::PUDGE),
                pick(1, Team::Dire, HeroId::SHADOW_FIEND),
            ];
            let map = map_of(MAP2_ID);
            let five_minutes = map.pregame_ticks + 5 * 60 * 30;
            let match_seed = game
                .seed(seed, MATCH_DOMAIN)
                .expect("a slot below MAX_SLOTS");
            MatchSpec::new(MAP2_ID, picks, match_seed, five_minutes)
        }),
    };
    let [radiant, dire] = map_of(MAP2_ID).fountains;
    let mut bots: Vec<SplitMix64> = (0..slots)
        .map(|slot| SplitMix64::new(bota_gym::derive_seed(seed, slot as u64, POLICY_DOMAIN)))
        .collect();
    std::thread::scope(|scope| {
        let pool = Workers::spawn(
            scope,
            WorkerConfig {
                threads: workers,
                queue: slots,
                cpus: None,
                name: "sim".to_owned(),
            },
            &config,
        )
        .expect("workers");
        let mut env: VecEnv<PlayerEnv<()>> =
            VecEnv::start(pool, (0..slots).collect(), &()).expect("the first games");
        let started = Instant::now();
        let mut ticks = 0u64;
        for _ in 0..decisions {
            let actions: Vec<SeatOrders> = env
                .episodes()
                .iter()
                .zip(&mut bots)
                .map(|(episode, bot)| {
                    let mut orders: SeatOrders = [None; MAX_SEATS];
                    for (seat, order) in orders.iter_mut().enumerate().take(episode.seat_count()) {
                        // Only what this seat's own client knows decides its order.
                        let knows_its_hero = episode.seat(seat).own_hero().is_some();
                        if knows_its_hero && bot.below(10).expect("a draw") == 0 {
                            let along = bot.below(1001).expect("a draw") as i32;
                            *order = Some(IssuedOrder::hero(Order::Attack {
                                target: Target::Pos(between(radiant, dire, along)),
                            }));
                        }
                    }
                    orders
                })
                .collect();
            ticks += 3 * slots as u64;
            env.step(actions, &(), |_, finished| {
                // The final statistics come with MatchOver; a game cut at its limit has none.
                let scores: Vec<String> = finished
                    .result
                    .iter()
                    .flat_map(|stats| &stats.slots)
                    .map(|seat| {
                        format!(
                            "{}/{}/{} lh {} dn {} gold {}",
                            seat.kills,
                            seat.deaths,
                            seat.assists,
                            seat.last_hits,
                            seat.denies,
                            seat.net_worth
                        )
                    })
                    .collect();
                println!(
                    "slot {} game {} ended at tick {} ({}): {}",
                    finished.game.slot,
                    finished.game.game,
                    finished.tick,
                    match finished.end.winner {
                        Some(winner) => format!("{winner:?} won"),
                        None => "cut at the limit".to_owned(),
                    },
                    scores.join(" | ")
                );
            })
            .expect("a step");
        }
        let millis = started.elapsed().as_millis().max(1);
        println!(
            "{slots} slots, {workers} workers: {} decisions/s, {} ticks/s",
            (decisions * slots) as u128 * 1000 / millis,
            u128::from(ticks) * 1000 / millis
        );
    });
}

/// The spot `along` thousandths of the way from `from` to `to`.
fn between(from: Vec2, to: Vec2, along: i32) -> Vec2 {
    let at = |a: i32, b: i32| a + (b - a) * along / 1000;
    Vec2::from_ints(
        at(from.x.to_int(), to.x.to_int()),
        at(from.y.to_int(), to.y.to_int()),
    )
}

fn arguments() -> (usize, usize, usize, u64) {
    let (mut slots, mut workers, mut decisions, mut seed) = (4, 2, 600, 1);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value: u64 = args
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| usage());
        match flag.as_str() {
            "--slots" => slots = value as usize,
            "--workers" => workers = value as usize,
            "--decisions" => decisions = value as usize,
            "--seed" => seed = value,
            _ => usage(),
        }
    }
    (slots, workers, decisions, seed)
}

fn usage() -> ! {
    eprintln!("usage: random_duel [--slots N] [--workers N] [--decisions N] [--seed N]");
    std::process::exit(2);
}
