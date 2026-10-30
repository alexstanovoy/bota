//! Shared vocabulary and wire format for the bota game protocol.
//!
//! Everything `bota_server` and `bota_client_core` exchange is defined
//! here. A type belongs in this crate when it crosses the wire, or when it is
//! needed to read what crosses the wire. What the server promises about the
//! exchange beyond the types is the [`contract`].

mod attrs;
mod chat;
mod codec;
pub mod contract;
mod event;
mod fingerprint;
mod ids;
pub mod limits;
mod match_rule;
mod math;
mod msg;
mod order;
mod registry;
mod unpack;
mod view;

#[cfg(test)]
mod tests;

pub use attrs::*;
pub use chat::*;
pub use codec::*;
pub use event::*;
pub use fingerprint::*;
pub use ids::*;
pub use limits::*;
pub use match_rule::*;
pub use math::*;
pub use msg::*;
pub use order::*;
pub use registry::*;
pub use unpack::*;
pub use view::*;
