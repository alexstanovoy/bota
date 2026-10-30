//! Writing the records of a metadata file into bytes.

use bota_proto::encode_frame_to_vec;

use super::record::MetaRecord;
use crate::limits::{END_RESERVE_BYTES, MAX_META_BYTES, MAX_RECORD_BYTES};

/// The frames of a file so far, held to [`MAX_META_BYTES`]: a record that would pass it is
/// dropped and the file is marked cut, except the last ones, which may use the reserve.
#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
    cut: bool,
}

impl Writer {
    /// Appends a record; false when it was dropped. `closing` records may use the reserve.
    pub fn push(&mut self, record: &MetaRecord, closing: bool) -> bool {
        let Ok(frame) = encode_frame_to_vec(record) else {
            return false;
        };
        let room = MAX_META_BYTES + if closing { END_RESERVE_BYTES } else { 0 };
        if frame.len() > MAX_RECORD_BYTES + bota_proto::LEN_PREFIX
            || self.bytes.len() + frame.len() > room
        {
            self.cut = true;
            return false;
        }
        self.bytes.extend_from_slice(&frame);
        true
    }

    /// Whether a record was dropped for want of room.
    pub fn cut(&self) -> bool {
        self.cut
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
