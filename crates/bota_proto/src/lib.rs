//! Shared vocabulary and wire format for the bota game protocol.
//!
//! Everything `bota_server` and `bota_client_core` exchange is defined
//! here. A type belongs in this crate when it crosses the wire, or when it is
//! needed to read what crosses the wire. What the server promises about the
//! exchange beyond the types is the [`contract`].

mod attrs;
mod bounded;
mod chat;
mod codec;
pub mod contract;
mod entity_ids;
mod event;
mod fingerprint;
pub mod game_limits;
mod ids;
pub mod limits;
mod match_rule;
mod math;
mod msg;
mod order;
mod peer;
mod registry;
mod status;
mod talents;
mod unpack;
mod view;

#[cfg(test)]
mod tests;

pub use attrs::*;
pub use chat::*;
pub use codec::*;
pub use event::*;
pub use fingerprint::*;
pub use game_limits::*;
pub use ids::*;
pub use limits::{
    MAX_ANNOUNCED_NAME_BYTES, MAX_CHAT_BYTES, MAX_CHAT_GRAPHEMES, MAX_CLIENT_PAYLOAD_LEN,
    MAX_DECODED_BYTES, MAX_EVENTS_PER_BATCH, MAX_PAYLOAD_LEN, MAX_SHOWN_NAME_BYTES,
};
pub use match_rule::*;
pub use math::*;
pub use msg::*;
pub use order::*;
pub use peer::*;
pub use registry::*;
pub use status::*;
pub use talents::*;
pub use unpack::*;
pub use view::*;
