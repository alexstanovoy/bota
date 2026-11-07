//! The match metadata file.
//!
//! Seat identities are optional public roster facts, never observer identities. Permanent
//! departures retain their server acceptance tick even when the match has no final result;
//! a reconnectable disconnection is not a departure. Old files omit both facts.
//!
//! A `StructureLocation` enriches the zero-based ordinal of a successfully written legacy
//! `Structure` record. The legacy record stays frozen, so older readers still see building
//! falls and newer readers distinguish simultaneous falls without guessing from kind or team.

mod read;
mod record;
mod recorder;
mod write;

pub use read::{Meta, MetaError, decode};
pub use record::*;
pub use recorder::Recorder;
pub use write::Writer;
