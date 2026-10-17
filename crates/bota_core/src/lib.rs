//! The rules of bota a player may know: balance tables and pure integer
//! functions of them.
//!
//! The server applies these rules to its world; a client applies the same
//! rules to what it sees. Nothing here holds a world, an entity handle, a
//! random stream, a clock or any I/O, so nothing here can reveal hidden
//! state: a hidden quantity enters a function only as a parameter.

#![forbid(unsafe_code)]

mod ability;
mod aggro;
mod attack_class;
mod aura;
mod camp;
mod cells;
mod chance;
mod damage;
mod economy;
mod effect;
mod hero;
pub mod heroes;
mod item;
mod item_table;
mod map;
mod modifier;
mod movement;
mod place;
mod progress;
mod protect;
mod regen;
mod roster;
pub mod rules;
mod sight;
mod standing;
mod talent;
mod terrain;
mod unit;
mod unit_kind;
mod unit_neutral;
mod wave;

pub use ability::*;
pub use aggro::*;
pub use attack_class::*;
pub use aura::*;
pub use camp::*;
pub use cells::*;
pub use chance::*;
pub use damage::*;
pub use economy::*;
pub use effect::*;
pub use hero::*;
pub use heroes::*;
pub use item::*;
pub use item_table::*;
pub use map::*;
pub use modifier::*;
pub use movement::*;
pub use place::*;
pub use progress::*;
pub use protect::*;
pub use regen::*;
pub use roster::*;
pub use sight::*;
pub use standing::*;
pub use talent::*;
pub use terrain::*;
pub use unit::*;
pub use unit_kind::*;
pub use unit_neutral::*;
pub use wave::*;
