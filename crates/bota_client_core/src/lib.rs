//! Everything a bota front end needs that is not drawing: the connection and
//! the replay player, the client's picture of the world and how snapshots
//! and events change it, motion between snapshots, what a key or a click
//! means as an order, and the derived facts the aids show.

mod catalog;
mod console;
mod derive;
mod events;
mod fog;
mod intent;
mod lasthit;
mod net;
mod replay_play;
mod shop;
mod slots;
mod smooth;
mod state;

#[cfg(test)]
mod tests;

pub use catalog::*;
pub use console::*;
pub use derive::*;
pub use events::*;
pub use fog::*;
pub use intent::*;
pub use lasthit::*;
pub use net::*;
pub use replay_play::*;
pub use shop::*;
pub use slots::*;
pub use smooth::*;
pub use state::*;
