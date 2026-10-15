//! The rules of bota a player may know: balance tables and pure integer
//! functions of them.
//!
//! The server applies these rules to its world; a client applies the same
//! rules to what it sees. Nothing here holds a world, an entity handle, a
//! random stream, a clock or any I/O, so nothing here can reveal hidden
//! state: a hidden quantity enters a function only as a parameter.
//!
//! See `DESIGN.md` for the architecture this follows from.

#![forbid(unsafe_code)]

mod attack_class;
mod cells;
mod chance;
mod movement;
mod progress;
pub mod rules;
mod sight;
mod talent;
mod unit_kind;
mod wave;

pub use attack_class::*;
pub use cells::*;
pub use chance::*;
pub use movement::*;
pub use progress::*;
pub use sight::*;
pub use talent::*;
pub use unit_kind::*;
pub use wave::*;
