//! Sockets, framing and per-connection threads.

mod conn;
mod outbox;

pub use conn::*;
pub use outbox::*;

#[cfg(test)]
mod tests;
