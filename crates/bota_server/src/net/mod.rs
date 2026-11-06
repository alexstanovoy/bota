//! Sockets, framing and per-connection threads.

mod allowance;
mod conn;
mod deflate;
mod inbound;
mod listeners;
mod outbox;
mod pending;
mod source;
mod stream;
#[cfg(feature = "websocket")]
mod ws;

pub use allowance::*;
pub use conn::*;
pub use inbound::{
    InboundBudget, MAX_QUEUED_CLIENT_MESSAGES, MAX_QUEUED_MESSAGES_PER_PEER, QueuedClientMessage,
};
pub use listeners::*;
pub use outbox::*;
pub use pending::MAX_PENDING_CONNECTIONS;
pub use source::*;
pub use stream::*;
#[cfg(feature = "websocket")]
pub use ws::*;

#[cfg(test)]
mod tests;
