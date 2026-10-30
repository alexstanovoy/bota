//! Reading a metadata file: every record it holds that this build knows, and a count of those
//! it does not.

use bota_proto::{LEN_PREFIX, decode_payload};

use super::record::{MetaHeader, MetaRecord};
use crate::limits::{MAX_META_BYTES, MAX_RECORD_BYTES};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetaError {
    /// More bytes than a file may hold.
    TooLong,
    /// The first record is no header of this format.
    NotMeta,
}

impl std::fmt::Display for MetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MetaError::TooLong => "a metadata file longer than a file may be",
            MetaError::NotMeta => "not a bota metadata file",
        })
    }
}

/// A file read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub header: MetaHeader,
    /// The records this build knows, in order, the header not among them.
    pub records: Vec<MetaRecord>,
    /// Frames it skipped: records a later build wrote.
    pub skipped: u32,
    /// Whether the match was decided in it (an `End` record): a file cut short, or of a match
    /// abandoned, has none.
    pub complete: bool,
}

/// Reads a file. A frame cut off at the end stops the reading with what came before; a frame
/// of a record this build does not know is skipped by its length.
pub fn decode(bytes: &[u8]) -> Result<Meta, MetaError> {
    if bytes.len() > MAX_META_BYTES + crate::limits::END_RESERVE_BYTES {
        return Err(MetaError::TooLong);
    }
    let mut frames = Frames(bytes);
    let header = match frames.next() {
        Some(Frame::Payload(payload)) => match decode_payload::<MetaRecord>(payload) {
            Ok(MetaRecord::Header(header)) if header.magic == MetaHeader::MAGIC => header,
            _ => return Err(MetaError::NotMeta),
        },
        _ => return Err(MetaError::NotMeta),
    };
    let mut meta = Meta {
        header,
        records: Vec::new(),
        skipped: 0,
        complete: false,
    };
    for frame in frames {
        let Frame::Payload(payload) = frame else {
            break;
        };
        match decode_payload::<MetaRecord>(payload) {
            Ok(MetaRecord::Header(_)) | Err(_) => meta.skipped += 1,
            Ok(record) => {
                meta.complete |= matches!(record, MetaRecord::End { .. });
                meta.records.push(record);
            }
        }
    }
    Ok(meta)
}

enum Frame<'a> {
    Payload(&'a [u8]),
    /// A frame cut off, or longer than a record may be.
    Broken,
}

struct Frames<'a>(&'a [u8]);

impl<'a> Iterator for Frames<'a> {
    type Item = Frame<'a>;

    fn next(&mut self) -> Option<Frame<'a>> {
        if self.0.is_empty() {
            return None;
        }
        let Some((head, rest)) = self.0.split_first_chunk::<LEN_PREFIX>() else {
            self.0 = &[];
            return Some(Frame::Broken);
        };
        let len = u32::from_le_bytes(*head) as usize;
        if len > MAX_RECORD_BYTES || len > rest.len() {
            self.0 = &[];
            return Some(Frame::Broken);
        }
        let (payload, rest) = rest.split_at(len);
        self.0 = rest;
        Some(Frame::Payload(payload))
    }
}
