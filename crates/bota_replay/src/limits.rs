//! Bounds on the metadata file: everything a reader of one holds is bounded by these, and a
//! writer stops at them.

/// Most bytes of a metadata file, as Dota's `.meta` is held to a megabyte.
pub const MAX_META_BYTES: usize = 1024 * 1024;
/// Most bytes of one record's payload.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
/// Bytes a file may go past [`MAX_META_BYTES`] by: room for its end and final records.
pub const END_RESERVE_BYTES: usize = 32 * 1024;
/// Most game minutes sampled: six hours.
pub const MAX_MINUTES: u32 = 360;
/// Most entries in one record's list of values.
pub const MAX_LIST: usize = 64;
/// Most hero bodies and buildings the recorder follows.
pub const MAX_TRACKED: usize = 65_536;
