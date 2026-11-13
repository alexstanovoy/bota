//! The rules of bota a player may know: balance tables and pure integer
//! functions of them.
//!
//! The server applies these rules to its world; a client applies the same
//! rules to what it sees. Nothing here holds a world, an entity handle, a
//! random stream, a clock or any I/O, so nothing here can reveal hidden
//! state: a hidden quantity enters a function only as a parameter.

#![forbid(unsafe_code)]

mod ability;
mod ability_neutral;
mod aggro;
mod aoe;
mod attack_class;
mod aura;
mod camp;
mod cast_rule;
mod catalogue;
mod cells;
mod chance;
pub mod creep_data;
mod damage;
mod data;
mod growth_clock;
mod lifesteal;
mod placement_data;
mod roshan_data;
pub use growth_clock::*;
pub use lifesteal::*;
pub use placement_data::DOTA_NO_WARDS;
mod ward;
pub use ward::*;
mod banner;
pub use banner::*;
mod disassembly;
mod economy;
mod effect;
mod fingerprint;
mod hero;
mod hero_data;
pub mod heroes;
mod heroes_data;
mod hit_count;
mod item;
mod item_data;
mod item_table;
mod lair;
mod lotus;
mod map;
mod modifier;
mod movement;
mod objective;
mod order_rules;
pub use order_rules::*;
mod place;
mod progress;
mod protect;
mod purchase;
mod rates;
mod regen;
mod roshan;
mod roster;
pub mod rules;
mod rune;
mod shop;
mod sight;
mod standing;
mod table;
mod talent;
mod terrain;
mod text;
mod time_of_day;
mod unit;
mod unit_kind;
mod unit_neutral;
mod wave;

#[cfg(test)]
mod tests;

pub use ability::*;
pub use ability_neutral::*;
pub use aggro::*;
pub use aoe::*;
pub use attack_class::*;
pub use aura::*;
pub use camp::*;
pub use cast_rule::*;
pub use catalogue::*;
pub use cells::*;
pub use chance::*;
pub use damage::*;
pub use disassembly::*;
pub use economy::*;
pub use effect::*;
pub use fingerprint::*;
pub use hero::*;
pub use hero_data::*;
pub use heroes::*;
pub use hit_count::*;
pub use item::*;
pub use item_data::*;
pub use item_table::*;
pub use lair::*;
pub use lotus::*;
pub use map::*;
pub use modifier::*;
pub use movement::*;
pub use objective::*;
pub use place::*;
pub use progress::*;
pub use protect::*;
pub use purchase::*;
pub use rates::*;
pub use regen::*;
pub use roshan::*;
pub use roster::*;
pub use rune::*;
pub use shop::*;
pub use sight::*;
pub use standing::*;
pub use talent::*;
pub use terrain::*;
pub use text::*;
pub use time_of_day::*;
pub use unit::*;
pub use unit_kind::*;
pub use unit_neutral::*;
pub use wave::*;
