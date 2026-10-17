//! The parts of a front end that draw or smooth motion: they use floats and
//! so sit behind the `ui` feature, outside the deterministic client.
#![allow(clippy::float_arithmetic)]

mod app;
mod console;
mod derive;
mod events;
mod intent;
mod replay;
mod shop;
mod slots;
mod smooth;

#[cfg(test)]
mod tests;

pub use app::*;
pub use console::*;
pub use derive::*;
pub use intent::*;
pub use replay::*;
pub use shop::*;
pub use slots::*;
pub use smooth::*;
