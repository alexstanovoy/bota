//! Everything a bota client needs that is not drawing.
//!
//! A bot is one call: [`Link::play`] connects, takes a seat and plays the
//! match to its end, asking a closure for orders once per tick. The parts it
//! is made of stand on their own: [`join`] takes a seat over any
//! [`Transport`], [`play`] drives a joined seat, a [`Seat`] turns the lobby
//! and the match start into a [`ClientSession`], and the session is the
//! client's picture of the match, folded from the server's messages, which
//! numbers the orders given and tracks what stands after the server's
//! answers.
//!
//! ```no_run
//! use bota_client_core::{IssuedOrder, Link, PlayError};
//! use bota_core::fountain_pos;
//! use bota_proto::{HeroId, Order, Target};
//!
//! fn main() -> Result<(), PlayError> {
//!     let outcome = Link::play("127.0.0.1:4455", "bot", HeroId::SHADOW_FIEND, |session| {
//!         let enemy = fountain_pos(session.map(), session.enemy());
//!         let attack = IssuedOrder::hero(Order::Attack { target: Target::Pos(enemy) });
//!         session.own_hero().map(|_| attack) // a dead hero takes no orders
//!     })?;
//!     println!("winner: {:?}", outcome.winner);
//!     Ok(())
//! }
//! ```
//!
//! The socket transports are left out for `wasm32`, where the embedder supplies a
//! [`Transport`] of its own.
//!
//! The `ui` feature adds the float-using, drawing-side parts of a front end:
//! the app state, motion between snapshots, what a key or a click means as an
//! order, the derived facts the aids show, and the replay player.

#![forbid(unsafe_code)]

mod blurb;
mod catalog;
mod handshake;
mod lasthit;
mod latency;
mod limits;
mod match_static;
mod observer;
mod orders;
mod play;
mod readiness;
mod seat;
mod session;
mod shop;
mod sight;
#[cfg(not(target_arch = "wasm32"))]
mod socket;
mod transport;
#[cfg(feature = "ui")]
mod ui;

pub use catalog::*;
pub use handshake::*;
pub use lasthit::*;
pub use latency::*;
pub use limits::*;
pub use match_static::*;
pub use observer::*;
pub use orders::*;
pub use play::*;
pub use readiness::*;
pub use seat::*;
pub use session::*;
pub use shop::*;
pub use sight::*;
#[cfg(not(target_arch = "wasm32"))]
pub use socket::*;
pub use transport::*;
#[cfg(feature = "ui")]
pub use ui::*;
