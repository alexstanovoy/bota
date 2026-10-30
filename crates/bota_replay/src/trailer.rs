//! The metadata file inside a replay, after the end of its compressed records.
//!
//! A replay is one Zstandard stream. The trailer is a Zstandard skippable frame after it
//! (RFC 8878: a decoder skips a frame whose magic is `0x184D2A50..=0x184D2A5F`), so a reader
//! that has never heard of the trailer plays the replay as before, and holds no record of it and
//! no change of the wire. Its payload is the metadata file, byte for byte, then the file's
//! length and [`MAGIC`]; a reader that wants the file reads the last bytes of the replay and
//! finds it without unpacking anything.

use std::io::{Read, Seek, SeekFrom};

use crate::limits::{END_RESERVE_BYTES, MAX_META_BYTES};

/// The magic number of the skippable frame.
pub const FRAME_MAGIC: u32 = 0x184D_2A50;
/// The last eight bytes of a replay with a trailer.
pub const MAGIC: [u8; 8] = *b"bota.mtf";
/// Bytes after the file inside the frame: its length, then the magic.
const TAIL: usize = 4 + MAGIC.len();

/// The trailer for a metadata file, to be written right after a replay's compressed stream.
/// None when the file is longer than a trailer may carry.
pub fn frame(meta: &[u8]) -> Option<Vec<u8>> {
    if meta.len() > MAX_META_BYTES + END_RESERVE_BYTES {
        return None;
    }
    let payload = u32::try_from(meta.len() + TAIL).ok()?;
    let mut out = Vec::with_capacity(8 + meta.len() + TAIL);
    out.extend(FRAME_MAGIC.to_le_bytes());
    out.extend(payload.to_le_bytes());
    out.extend(meta);
    out.extend(u32::try_from(meta.len()).ok()?.to_le_bytes());
    out.extend(MAGIC);
    Some(out)
}

/// The metadata file a replay carries, if it carries one: read from its last bytes. A replay
/// without a trailer, or with one that does not hold together, gives none.
pub fn find(replay: &mut (impl Read + Seek)) -> std::io::Result<Option<Vec<u8>>> {
    let end = replay.seek(SeekFrom::End(0))?;
    if end < (8 + TAIL) as u64 {
        return Ok(None);
    }
    replay.seek(SeekFrom::Start(end - TAIL as u64))?;
    let mut tail = [0u8; TAIL];
    replay.read_exact(&mut tail)?;
    let (len, magic) = tail.split_at(4);
    let len = u32::from_le_bytes(len.try_into().expect("four bytes")) as usize;
    if magic != MAGIC || len > MAX_META_BYTES + END_RESERVE_BYTES {
        return Ok(None);
    }
    let start = end.checked_sub((8 + len + TAIL) as u64);
    let Some(start) = start else {
        return Ok(None);
    };
    replay.seek(SeekFrom::Start(start))?;
    let mut head = [0u8; 8];
    replay.read_exact(&mut head)?;
    let declared = u32::from_le_bytes(head[4..].try_into().expect("four bytes")) as usize;
    if head[..4] != FRAME_MAGIC.to_le_bytes() || declared != len + TAIL {
        return Ok(None);
    }
    let mut meta = vec![0u8; len];
    replay.read_exact(&mut meta)?;
    Ok(Some(meta))
}
