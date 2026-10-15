//! Shared vocabulary and wire format for the bota game protocol.
//!
//! Everything `bota_server` and `bota_client_core` exchange is defined
//! here. A type belongs in this crate when it crosses the wire, or when it is
//! needed to read what crosses the wire.
//!
//! See `DESIGN.md` for the architecture this follows from.

mod attrs;
mod codec;
mod event;
mod ids;
mod math;
mod msg;
mod order;
mod registry;
mod view;

#[cfg(test)]
mod tests;

pub use attrs::*;
pub use codec::*;
pub use event::*;
pub use ids::*;
pub use math::*;
pub use msg::*;
pub use order::*;
pub use registry::*;
pub use view::*;
