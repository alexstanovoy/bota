//! Simulation and network server for bota.
//!
//! [`engine`] is how state is kept and walked and knows nothing of Dota;
//! [`game`] is the game itself and knows nothing about sockets. Everything a
//! participant is allowed to see leaves through `bota-proto`; everything else,
//! including the randomness, stays here.
//!
//! See `DESIGN.md` for the architecture this follows from.

pub mod engine;
pub mod game;
pub mod game_loop;
pub mod lobby;
pub mod net;
pub mod replay;

#[cfg(feature = "phase-profile")]
pub mod profile;
#[cfg(all(test, feature = "phase-profile"))]
mod profile_tests;
