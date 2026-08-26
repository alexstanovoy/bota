//! A bot that decides by naming one of a fixed list of deeds.
//!
//! The contract is three pieces and nothing else. [`Field`] reads a tick into
//! a settled shape; [`sight`] turns that into the numbers a model is shown;
//! [`Deed`] is the numbered list of what may be chosen, with a flag per deed
//! saying whether it could be done and one way of turning a choice into an
//! order.
//!
//! Between them sits a [`Mind`], which is handed numbers and flags and answers
//! with one number. That is the whole of the seam: a mind that knows what a
//! creep is has reached across it, and a game that knows what a weight is has
//! reached back. [`Learned`] is the mind that is a network; the rest of the
//! crate would not notice another.

mod ask;
#[cfg(feature = "builtin")]
mod bench;
mod breed;
mod deed;
mod doing;
mod field;
mod lane;
mod link;
mod marks;
mod mind;
mod model;
mod plan;
mod seat;
mod shop;
mod sight;
mod spells;
mod swiss;
mod wire;
mod yard;

pub use ask::*;
#[cfg(feature = "builtin")]
pub use bench::*;
pub use breed::*;
pub use deed::*;
pub use doing::*;
pub use field::*;
pub use lane::*;
pub use link::*;
pub use marks::*;
pub use mind::*;
pub use model::*;
pub use plan::*;
pub use seat::*;
pub use shop::*;
pub use sight::*;
pub use spells::*;
pub use swiss::*;
pub use wire::*;
pub use yard::*;

#[cfg(test)]
mod tests;
