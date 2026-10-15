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

mod aggro;
mod attack_class;
mod aura;
mod camp;
mod cells;
mod chance;
mod economy;
mod map;
mod modifier;
mod movement;
mod place;
mod progress;
mod protect;
mod roster;
pub mod rules;
mod sight;
mod talent;
mod unit;
mod unit_kind;
mod unit_neutral;
mod wave;

pub use aggro::*;
pub use attack_class::*;
pub use aura::*;
pub use camp::*;
pub use cells::*;
pub use chance::*;
pub use economy::*;
pub use map::*;
pub use modifier::*;
pub use movement::*;
pub use place::*;
pub use progress::*;
pub use protect::*;
pub use roster::*;
pub use sight::*;
pub use talent::*;
pub use unit::*;
pub use unit_kind::*;
pub use unit_neutral::*;
pub use wave::*;
