//! Writing the replay file.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use bota_proto::{ReplayHeader, ReplayRecord, encode_frame_to_vec};

/// Appends [`ReplayRecord`] frames to a `.brp` file.
///
/// A write error poisons the writer: recording stops with a line on stderr,
/// the match goes on.
pub struct ReplayWriter {
    out: Option<BufWriter<File>>,
}

impl ReplayWriter {
    /// Creates or truncates the file and starts it with this build's
    /// [`ReplayHeader`]; an error names the path.
    pub fn create(path: &Path) -> std::io::Result<ReplayWriter> {
        let file = File::create(path).map_err(|error| {
            std::io::Error::new(error.kind(), format!("replay {}: {error}", path.display()))
        })?;
        let mut writer = ReplayWriter {
            out: Some(BufWriter::new(file)),
        };
        writer.record(&ReplayRecord::Header(ReplayHeader::CURRENT));
        Ok(writer)
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
        let written = encode_frame_to_vec(record)
            .map_err(|error| error.to_string())
            .and_then(|frame| out.write_all(&frame).map_err(|error| error.to_string()));
        if let Err(error) = written {
            eprintln!("bota_server: the replay stops here ({error})");
            self.out = None;
        }
    }

    /// Flushes everything to disk.
    pub fn finish(mut self) {
        if let Some(out) = self.out.as_mut()
            && let Err(error) = out.flush()
        {
            eprintln!("bota_server: the replay's end was not written ({error})");
        }
    }
}
