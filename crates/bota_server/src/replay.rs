//! Writing the replay file.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use bota_proto::{ReplayHeader, ReplayRecord, encode_frame_to_vec};
use zstd::stream::write::Encoder;

/// The Zstandard level a replay is written at: a match's records shrink some
/// 87 times, at well under a millisecond of a core per second of play.
const LEVEL: i32 = 3;

/// The Zstandard window, as a power of two: 2 MiB, what level 3 takes for a
/// stream of unknown length, well inside what a player accepts.
const WINDOW_LOG: u32 = 21;

/// Appends [`ReplayRecord`] frames to a `.brp` file, as one Zstandard stream.
///
/// A write error poisons the writer: recording stops with a line on stderr,
/// the match goes on.
pub struct ReplayWriter {
    out: Option<Encoder<'static, BufWriter<File>>>,
}

impl ReplayWriter {
    /// Creates or truncates the file and starts it with this build's
    /// [`ReplayHeader`]; an error names the path.
    pub fn create(path: &Path) -> std::io::Result<ReplayWriter> {
        let file = File::create(path).map_err(|error| {
            std::io::Error::new(error.kind(), format!("replay {}: {error}", path.display()))
        })?;
        let mut encoder = Encoder::new(BufWriter::new(file), LEVEL)?;
        encoder.window_log(WINDOW_LOG)?;
        let mut writer = ReplayWriter { out: Some(encoder) };
        writer.record(&ReplayRecord::Header(ReplayHeader::new(
            bota_core::WIRE_FINGERPRINT,
        )));
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

    /// Ends the stream and flushes everything to disk.
    pub fn finish(mut self) {
        self.end();
    }

    fn end(&mut self) {
        let Some(out) = self.out.take() else {
            return;
        };
        if let Err(error) = out.finish().and_then(|mut file| file.flush()) {
            eprintln!("bota_server: the replay's end was not written ({error})");
        }
    }
}

/// A writer let go without [`ReplayWriter::finish`] (a lobby abandoned, a
/// panic unwinding) still ends its stream, so the file unpacks.
impl Drop for ReplayWriter {
    fn drop(&mut self) {
        self.end();
    }
}
