//! A bot that plays by rules rather than by weights.
//!
//! Two pieces sit here. [`Bot`] and [`play`] are the seam: whatever holds a
//! seat is handed one tick at a time and answers with at most one [`Ask`],
//! and the loop between that and a socket is written once. [`Playbook`] is
//! the bot this crate ships — a ladder of wants walked top to bottom, with
//! spellwork for Shadow Fiend and for Sylla.
//!
//! Nothing in it is drawn at random, so a match played twice against the same
//! opponent goes the same way both times.
//!
//! See `DESIGN.md` for the architecture this follows from.

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
