//! The metadata file of a replay: the one it carries, or the one its records give.

use std::io::{self, Read};

use bota_proto::{FrameReader, ReplayRecord};
use bota_replay::meta::Recorder;

use crate::replay_file::UnpackedReplay;

/// The metadata file of the match a replay holds, written by the recorder over its records: the
/// same bytes the server wrote while the match ran, if the replay is whole. `reader` is the
/// replay as a file, packed or not. A replay of another wire, or one that cannot be read, is an
/// error; one cut short gives the file of the match up to there.
pub fn meta_of_replay(reader: impl Read + 'static) -> io::Result<Vec<u8>> {
    let mut frames = FrameReader::new();
    let mut recorder = Recorder::new();
    let mut unpacked = UnpackedReplay::new(reader);
    let mut chunk = vec![0u8; 64 * 1024];
    let mut header = false;
    let mut read = Ok(());
    loop {
        match unpacked.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => frames.push(&chunk[..n]),
            Err(error) => {
                read = Err(error);
                break;
            }
        }
        while let Some(record) = frames
            .next_message::<ReplayRecord>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?
        {
            match &record {
                ReplayRecord::Header(found) if !header => {
                    if found.magic != bota_proto::ReplayHeader::MAGIC
                        || found.wire_fingerprint != bota_core::WIRE_FINGERPRINT
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "a replay of another wire",
                        ));
                    }
                    header = true;
                }
                _ if !header => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "a replay without its header",
                    ));
                }
                _ => recorder.push(&record),
            }
        }
    }
    // A replay cut short is read as far as it goes.
    if !header {
        return read.and(Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an empty replay",
        )));
    }
    Ok(recorder.finish())
}

/// The metadata file a replay file carries at its end, if it carries one.
pub fn meta_in_replay(replay: &mut (impl Read + io::Seek)) -> io::Result<Option<Vec<u8>>> {
    bota_replay::trailer::find(replay)
}
