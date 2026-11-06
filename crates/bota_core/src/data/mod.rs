//! Bota's game data: what Dota's scripts give each item, generated from them, and the rows
//! the catalog makes of it.

mod item_mechanics;
pub(crate) mod items;
mod placement;
mod roshan;
pub use placement::*;
pub(crate) use roshan::*;

pub(crate) use item_mechanics::ITEM_ROWS;
