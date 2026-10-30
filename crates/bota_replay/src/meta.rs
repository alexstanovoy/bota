//! The match metadata file.

mod read;
mod record;
mod recorder;
mod write;

pub use read::{Meta, MetaError, decode};
pub use record::*;
pub use recorder::Recorder;
pub use write::Writer;
