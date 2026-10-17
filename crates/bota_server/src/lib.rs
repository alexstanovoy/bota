//! Simulation and network server for bota.
//!
//! [`engine`] is how state is kept and walked and knows nothing of Dota;
//! [`game`] is the game itself and knows nothing about sockets. Everything a
//! participant is allowed to see leaves through `bota_proto`; everything else,
//! including the randomness, stays here.

pub mod engine;
pub mod game;
pub mod game_loop;
pub mod lobby;
pub mod local;
pub mod net;
pub mod profile;
pub mod replay;
