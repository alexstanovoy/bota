//! Writing the replay file.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bota_proto::{LEN_PREFIX, MAX_PAYLOAD_LEN, ReplayHeader, ReplayRecord, encode_frame_to_vec};
use zstd::stream::write::Encoder;

/// The Zstandard level a replay is written at: a match's records shrink some
/// 87 times, at well under a millisecond of a core per second of play.
const LEVEL: i32 = 3;

/// The Zstandard window, as a power of two: 2 MiB, what level 3 takes for a
/// stream of unknown length, well inside what a player accepts.
const WINDOW_LOG: u32 = 21;

/// The room left for the stream's end when a size cap closes the replay: what the
/// compressor still holds (one block) and the end of the frame. A cap under four times this
/// keeps a quarter of itself.
const CLOSE_MARGIN: u64 = 256 << 10;

/// The file under the encoder, counting the bytes that reached it.
struct Counted {
    file: File,
    written: Arc<AtomicU64>,
}

impl Write for Counted {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(buf)?;
        self.written.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Appends [`ReplayRecord`] frames to a `.brp` file, as one Zstandard stream.
///
/// Recording stops, with a line on stderr, when a write fails or the file nears its size cap;
/// the match goes on. At the cap the stream is closed whole, so the replay plays up to the
/// last record it holds. [`ReplayWriter::truncated`] tells that records were left out.
pub struct ReplayWriter {
    out: Option<Encoder<'static, BufWriter<Counted>>>,
    written: Arc<AtomicU64>,
    /// The file's size cap in bytes; absent for none.
    max_bytes: Option<u64>,
    truncated: bool,
}

impl ReplayWriter {
    /// Creates or truncates the file and starts it with this build's
    /// [`ReplayHeader`]; an error names the path. The file is closed before it grows past
    /// `max_bytes`, when given.
    pub fn create(path: &Path, max_bytes: Option<u64>) -> std::io::Result<ReplayWriter> {
        let file = File::create(path).map_err(|error| {
            std::io::Error::new(error.kind(), format!("replay {}: {error}", path.display()))
        })?;
        let written = Arc::new(AtomicU64::new(0));
        let counted = Counted {
            file,
            written: Arc::clone(&written),
        };
        let mut encoder = Encoder::new(BufWriter::new(counted), LEVEL)?;
        encoder.window_log(WINDOW_LOG)?;
        let mut writer = ReplayWriter {
            out: Some(encoder),
            written,
            max_bytes,
            truncated: false,
        };
        writer.record(&ReplayRecord::Header(ReplayHeader::new(
            bota_core::WIRE_FINGERPRINT,
        )));
        Ok(writer)
    }

    /// A writer that records nothing.
    pub fn disabled() -> ReplayWriter {
        ReplayWriter {
            out: None,
            written: Arc::new(AtomicU64::new(0)),
            max_bytes: None,
            truncated: false,
        }
    }

    /// Whether records were left out of the file: a write failed or the size cap was reached.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Appends one record.
    pub fn record(&mut self, record: &ReplayRecord) {
        let Some(out) = self.out.as_mut() else {
            return;
        };
        let written = encode_frame_to_vec(record)
            .map_err(|error| error.to_string())
            .and_then(|frame| out.write_all(&frame).map_err(|error| error.to_string()));
        self.after_write(written);
    }

    /// Appends the [`ReplayRecord::Msg`] carrying the server message an
    /// encoded frame holds, without encoding the message again: the bytes
    /// [`ReplayWriter::record`] would write for it.
    pub fn record_msg_frame(&mut self, frame: &[u8]) {
        let Some(out) = self.out.as_mut() else {
            return;
        };
        let written = match msg_record_frame(frame) {
            Some((head, payload)) => out
                .write_all(&head)
                .and_then(|()| out.write_all(payload))
                .map_err(|error| error.to_string()),
            None => Err(format!("a frame of {} bytes holds no record", frame.len())),
        };
        self.after_write(written);
    }

    /// Stops recording when a write failed (the stream cannot be closed then) and closes the
    /// stream whole when the file is near its cap.
    fn after_write(&mut self, written: Result<(), String>) {
        if let Err(error) = written {
            eprintln!("bota_server: the replay stops here ({error})");
            self.out = None;
            self.truncated = true;
            return;
        }
        let Some(max) = self.max_bytes else {
            return;
        };
        let margin = CLOSE_MARGIN.min(max / 4);
        if self.written.load(Ordering::Relaxed) + margin >= max {
            eprintln!("bota_server: the replay is closed here, at its size cap of {max} bytes");
            self.end();
            self.truncated = true;
        }
    }

    /// Ends the stream and flushes everything to disk; whether records were left out.
    pub fn finish(mut self) -> bool {
        self.end();
        self.truncated
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

/// The variant index of [`ReplayRecord::Msg`], the first of the enum: postcard
/// writes it as a one-byte varint ahead of the message it carries.
const MSG_RECORD: u8 = 0;

/// The frame of the [`ReplayRecord::Msg`] carrying the message of a
/// [`ServerMsg`](bota_proto::ServerMsg) frame, as the head to write before the
/// message's payload and that payload. None for a frame cut short or one whose
/// record would be over [`MAX_PAYLOAD_LEN`].
fn msg_record_frame(frame: &[u8]) -> Option<([u8; LEN_PREFIX + 1], &[u8])> {
    let payload = frame.get(LEN_PREFIX..)?;
    let len = payload.len() + 1;
    if len > MAX_PAYLOAD_LEN {
        return None;
    }
    let mut head = [MSG_RECORD; LEN_PREFIX + 1];
    head[..LEN_PREFIX].copy_from_slice(&u32::try_from(len).ok()?.to_le_bytes());
    Some((head, payload))
}

/// A writer let go without [`ReplayWriter::finish`] (a lobby abandoned, a
/// panic unwinding) still ends its stream, so the file unpacks.
impl Drop for ReplayWriter {
    fn drop(&mut self) {
        self.end();
    }
}

#[cfg(test)]
mod tests {
    use bota_proto::{EventKind, ReplayRecord, ServerMsg, encode_frame_to_vec};

    use super::msg_record_frame;

    /// A message recorded from its frame is the record a fresh encode
    /// writes, byte for byte.
    #[test]
    fn a_message_recorded_from_its_frame_is_the_record_encoded_afresh() {
        for msg in [
            ServerMsg::Events {
                tick: 300,
                events: vec![EventKind::TreeGrew {
                    at: bota_proto::Vec2::from_ints(512, 768),
                }],
            },
            ServerMsg::Snapshot {
                view: crate::game::World::new().view_full(),
            },
        ] {
            let frame = encode_frame_to_vec(&msg).expect("a message encodes");
            let (head, payload) = msg_record_frame(&frame).expect("a record");
            let recorded = [&head[..], payload].concat();
            let fresh = encode_frame_to_vec(&ReplayRecord::Msg(msg)).expect("a record encodes");
            assert_eq!(recorded, fresh);
        }
    }
}
