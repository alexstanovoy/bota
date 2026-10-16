//! Timing the regions of a tick: sampled and emitted with the `phase-profile`
//! feature, compiled to nothing without it.

mod phase;
#[cfg(feature = "phase-profile")]
mod record;
#[cfg(not(feature = "phase-profile"))]
mod silent;

#[cfg(all(test, feature = "phase-profile"))]
mod tests;

pub use phase::*;
#[cfg(feature = "phase-profile")]
pub use record::*;
#[cfg(not(feature = "phase-profile"))]
pub use silent::*;
