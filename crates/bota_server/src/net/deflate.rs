//! The compressed stream of one connection ([`Codec::Deflate`]): raw
//! DEFLATE with one window for the whole connection, flushed after every
//! entry the writer sends, so the client unpacks each entry whole as soon as
//! its last part arrives.
//!
//! [`Codec::Deflate`]: bota_proto::Codec::Deflate

use bota_proto::{ServerMsg, encode_frame};
use miniz_oxide::deflate::core::{
    CompressorOxide, TDEFLFlush, TDEFLStatus, compress, create_comp_flags_from_zip_params,
};

/// zlib's default level: on a match's snapshots a third smaller than level
/// one at twice its time, some twenty microseconds a tick.
const LEVEL: i32 = 6;

/// Raw DEFLATE, without a zlib header: the window is the format's 32 KiB.
const WINDOW_BITS: i32 = -15;

/// Compressed bytes one [`ServerMsg::Compressed`] carries at most.
pub const MAX_PART_LEN: usize = 64 * 1024;

/// One connection's compressor.
pub struct Deflater {
    compressor: Box<CompressorOxide>,
    part: Box<[u8]>,
}

impl Deflater {
    pub fn new() -> Deflater {
        let flags = create_comp_flags_from_zip_params(LEVEL, WINDOW_BITS, 0);
        Deflater {
            compressor: Box::new(CompressorOxide::new(flags)),
            part: vec![0; MAX_PART_LEN].into_boxed_slice(),
        }
    }

    /// The frames that carry `plain`, whole frames of server messages, as
    /// parts of the stream flushed at its end; nothing when the compressor
    /// failed, which leaves the stream unusable.
    pub fn pack(&mut self, plain: &[u8]) -> Option<Vec<u8>> {
        let mut frames = Vec::new();
        let mut rest = plain;
        loop {
            let (status, used, written) =
                compress(&mut self.compressor, rest, &mut self.part, TDEFLFlush::Sync);
            if status != TDEFLStatus::Okay {
                return None;
            }
            rest = &rest[used..];
            if written > 0 {
                let part = ServerMsg::Compressed(self.part[..written].to_vec());
                encode_frame(&part, &mut frames).ok()?;
            }
            if rest.is_empty() && written < self.part.len() {
                return Some(frames);
            }
        }
    }
}
