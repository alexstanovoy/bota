//! Everything a bota front end needs that is not drawing: the connection and
//! the replay player, the client's picture of the world and how snapshots
//! and events change it, motion between snapshots, what a key or a click
//! means as an order, and the derived facts the aids show.
//!
//! The `ui` feature adds the float-using, drawing-side parts.

#![forbid(unsafe_code)]

mod blurb;
mod catalog;
mod handshake;
mod latency;
mod limits;
mod match_static;
mod observer;
mod orders;
mod readiness;
mod session;
mod shop;
mod sight;
mod transport;
#[cfg(feature = "ui")]
mod ui;

pub use catalog::*;
pub use handshake::*;
pub use latency::*;
pub use limits::*;
pub use match_static::*;
pub use observer::*;
pub use orders::*;
pub use readiness::*;
pub use session::*;
pub use shop::*;
pub use sight::*;
pub use transport::*;
#[cfg(feature = "ui")]
pub use ui::*;
