//! Writing the replay file.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use bota_proto::{ReplayRecord, encode_frame_to_vec};

/// Appends [`ReplayRecord`] frames to a `.brp` file.
///
/// A write error poisons the writer: recording stops, the match does not.
pub struct ReplayWriter {
    out: Option<BufWriter<File>>,
}

impl ReplayWriter {
    /// Creates or truncates the file.
    pub fn create(path: &Path) -> std::io::Result<ReplayWriter> {
        Ok(ReplayWriter {
            out: Some(BufWriter::new(File::create(path)?)),
        })
    }

    /// A writer that records nothing.
    pub fn disabled() -> ReplayWriter {
        ReplayWriter { out: None }
    }

    /// Appends one record.
    pub fn record(&mut self, record: &ReplayRecord) {
        let Some(out) = self.out.as_mut() else {
            return;
        };
        let ok = encode_frame_to_vec(record)
            .ok()
            .and_then(|frame| out.write_all(&frame).ok())
            .is_some();
        if !ok {
            self.out = None;
        }
    }

    /// Flushes everything to disk.
    pub fn finish(mut self) {
        if let Some(out) = self.out.as_mut() {
            let _ = out.flush();
        }
    }
}
