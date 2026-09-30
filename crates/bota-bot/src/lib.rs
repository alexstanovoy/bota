//! A bot that plays by rules rather than by weights.
//!
//! [`Bot`] and [`play`] are the seam: whatever holds a seat is handed one tick
//! at a time and answers with at most one [`Ask`]. [`Playbook`] is the bot this
//! crate ships: a ladder of wants walked top to bottom, with spellwork for
//! Shadow Fiend and for Sylla. Nothing in it is drawn at random.

mod aim;
mod ask;
mod beat;
mod bot;
mod field;
mod fiend;
mod forest;
mod lane;
mod link;
mod numbers;
mod policy;
mod shop;
mod study;
mod sylla;
mod want;

pub use aim::*;
pub use ask::*;
pub use beat::*;
pub use bot::*;
pub use field::*;
pub use fiend::*;
pub use forest::*;
pub use lane::*;
pub use link::*;
pub use numbers::*;
pub use policy::*;
pub use shop::*;
pub use study::*;
pub use sylla::*;
pub use want::*;

#[cfg(test)]
mod tests;
