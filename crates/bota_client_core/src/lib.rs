//! Everything a bota front end needs that is not drawing: the connection and
//! the replay player, the client's picture of the world and how snapshots
//! and events change it, motion between snapshots, what a key or a click
//! means as an order, and the derived facts the aids show.
//!
//! The `ui` feature adds the float-using, drawing-side parts.

#![forbid(unsafe_code)]

mod catalog;
mod net;
#[cfg(feature = "ui")]
mod ui;

pub use catalog::*;
pub use net::*;
#[cfg(feature = "ui")]
pub use ui::*;
