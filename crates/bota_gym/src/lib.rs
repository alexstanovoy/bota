//! Train and evaluate bots against bota in-process.
//!
//! Everything a bot writer would otherwise build around [`bota_server::local::LocalMatch`]
//! before the first experiment: many concurrent matches stepped on a pool of workers, a
//! seeding scheme that makes runs replay exactly, opponent scheduling, batch runners for
//! evaluation, and per-seat statistics. The gym knows no neural network, tensor library or
//! training algorithm: the caller owns the policy.
//!
//! # The pieces
//!
//! - [`Game`]: one lockstep match. Each seat gets exactly the stream its connection to a server
//!   would carry, and gives one order a tick or a batch of them ([`Game::step_batches`]: clients
//!   may issue several orders per turn); [`MatchSpec`] picks the map, heroes, rules, order
//!   delays, a scenario's setup cheats and the tick limit. The hidden state is behind [`Game::truth`], which exists only on
//!   `Game<Privileged>`: what a policy may read and what only training signals may read are
//!   different types.
//! - [`Episode`]: the caller's code for one game of one slot (its match and whatever its seats'
//!   clients make of their streams), run on the [`Workers`]. [`PlayerEnv`] is a ready-made one:
//!   each seat's [`ClientSession`](bota_client_core::ClientSession) folds its stream, the policy
//!   reads the sessions and answers with one order per seat.
//! - [`VecEnv`]: a fixed set of slots stepped together, each slot playing game after game
//!   (the next one starts on the worker, in the same job that finished the last). For training.
//! - [`Tournament`]: a finite plan of games, a batch at a time, finished games replaced from the
//!   plan. For evaluation with a batched policy. [`paired_games`] plans every opponent on both
//!   sides of every seed.
//! - [`run_indexed`]: whole games as parallel jobs, for bots that need no batching.
//! - [`opponents`]: weighted opponent mixtures drawn per game from its seed, recent outcomes,
//!   and prioritised scheduling by win rate.
//! - [`seed`]: the seeding scheme.
//!
//! Everything but the matches themselves ([`Game`], [`MatchSpec`], [`PlayerEnv`]: the default
//! `simulation` feature) is plain Rust over `std`, for a crate that wants the plumbing without the
//! simulator.
//!
//! # Determinism
//!
//! A run is a pure function of its seeds: a match's randomness comes from its spec's seed, every
//! episode runs on whichever worker is free but owns all its state, the shared state is
//! read-only, and drivers put results in position order. The same seeds give the same
//! trajectories whatever the worker count, queue size, pinning or machine load.
//!
//! # Bounds
//!
//! Every collection and loop has a documented upper bound ([`MAX_SLOTS`], [`MAX_THREADS`],
//! [`MAX_QUEUE`], [`MAX_PARALLEL`], [`MAX_PLANS`], [`MAX_ADVANCE_TICKS`], [`MAX_TICK_LIMIT`]…).
//! Drivers allocate at start; a step only moves episodes and actions through pre-sized
//! channels. Caller code must be bounded too: see [`workers`].
//!
//! # Example
//!
//! Random bots on both seats of concurrent mid-only duels (`examples/random_duel.rs`):
//!
//! ```no_run
#![doc = include_str!("../examples/random_duel.rs")]
//! ```
//!
//! Run it with `cargo run --release -p bota_gym --example random_duel`; `examples/privileged_read.rs`
//! shows a training signal read through `Game<Privileged>` beside a policy that cannot.

mod error;
#[cfg(feature = "simulation")]
mod game;
pub mod opponents;
mod parallel;
#[cfg(feature = "simulation")]
mod player_env;
pub mod seed;
#[cfg(feature = "simulation")]
mod spec;
pub mod topology;
mod tournament;
mod vec_env;
pub mod workers;

pub use error::GymError;
#[cfg(feature = "simulation")]
pub use game::{
    Access, Advance, Game, MAX_ADVANCE_TICKS, MAX_ORDERS_PER_TICK, MAX_SEATS, Player, Privileged,
    Truth,
};
pub use parallel::run_indexed;
#[cfg(feature = "simulation")]
pub use player_env::{
    Finished, PlayerEnv, PlayerEnvConfig, SeatActions, SeatBatches, SeatOrders, SpecFn,
};
pub use seed::{
    GameId, MAX_PLANS, MAX_SLOTS, Pairing, SplitMix64, derive_seed, paired_games, seat_seed,
};
#[cfg(feature = "simulation")]
pub use spec::{MAX_SETUP_CHEATS, MAX_TICK_LIMIT, MatchSpec, SetupCheat};
pub use tournament::{MAX_PARALLEL, Tournament};
pub use vec_env::VecEnv;
pub use workers::{Episode, MAX_QUEUE, MAX_THREADS, WorkerConfig, Workers};
