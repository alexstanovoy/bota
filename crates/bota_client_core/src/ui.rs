//! The parts of a front end that draw or smooth motion: they use floats and
//! so sit behind the `ui` feature, outside the deterministic client.
#![allow(clippy::float_arithmetic)]

mod app;
mod console;
mod derive;
mod events;
mod fog;
mod intent;
mod lasthit;
mod replay;
mod shop;
mod slots;
mod smooth;

#[cfg(test)]
mod tests;

pub use app::*;
pub use console::*;
pub use derive::*;
pub use events::*;
pub use fog::*;
pub use intent::*;
pub use lasthit::*;
pub use replay::*;
pub use shop::*;
pub use slots::*;
pub use smooth::*;
