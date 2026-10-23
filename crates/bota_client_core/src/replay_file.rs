//! The frames of a `.brp` file a server wrote: one Zstandard stream, read
//! back by a pure-Rust decoder (in the browser too). A replay is untrusted
//! input, so what it may make a player hold and do is bounded.

use std::io::{self, ErrorKind, Read};

use bota_proto::MAX_PAYLOAD_LEN;
use ruzstd::decoding::{FrameDecoder, StreamingDecoder};

/// The bytes a Zstandard stream starts with. No replay of plain frames
/// starts with them: its first frame is a header of a few bytes, and so
/// its length prefix is small.
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// The largest window a replay's stream may ask a player to keep, in bytes:
/// four times what a server writes with, so a file packed again by hand at
/// a high level still plays.
pub const MAX_REPLAY_WINDOW: u64 = 8 << 20;

/// A replay unpacks to at most this many times the bytes it took from the
/// file, plus one frame: a server's replay unpacks some 87 times, and a
/// file that unpacks far beyond is a bomb.
pub const MAX_REPLAY_INFLATION: u64 = 1024;

/// The frames of a packed replay, unpacked as they are read.
pub struct UnpackedReplay {
    state: State,
}

enum State {
    /// Nothing read yet: the stream's header is read with the first read.
    Fresh(Box<dyn Read>),
    Open(Box<StreamingDecoder<Counted, FrameDecoder>>),
    /// The stream is spent, or broke: nothing more comes.
    Ended,
}

/// The file under the decoder, counting what the decoder took of it.
struct Counted {
    file: Box<dyn Read>,
    taken: u64,
    /// Bytes unpacked so far, against the bomb bound.
    unpacked: u64,
}

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read(buf)?;
        self.taken += read as u64;
        Ok(read)
    }
}

impl UnpackedReplay {
    /// The frames of the stream `file` holds from its first byte on. Nothing
    /// is read until this is.
    pub fn new(file: impl Read + 'static) -> UnpackedReplay {
        UnpackedReplay {
            state: State::Fresh(Box::new(file)),
        }
    }

    fn open(file: Box<dyn Read>) -> io::Result<State> {
        let counted = Counted {
            file,
            taken: 0,
            unpacked: 0,
        };
        let decoder = StreamingDecoder::new_with_max_window_size(counted, MAX_REPLAY_WINDOW)
            .map_err(|error| invalid(format!("the replay's stream does not open: {error}")))?;
        Ok(State::Open(Box::new(decoder)))
    }
}

/// Reads from an open stream, within the bomb bound; at its end, refuses
/// bytes after it.
fn read_open(
    decoder: &mut StreamingDecoder<Counted, FrameDecoder>,
    buf: &mut [u8],
) -> io::Result<usize> {
    let read = decoder
        .read(buf)
        .map_err(|error| invalid(format!("the replay's stream is broken: {error}")))?;
    let counted = decoder.get_mut();
    counted.unpacked += read as u64;
    let most = counted
        .taken
        .saturating_mul(MAX_REPLAY_INFLATION)
        .saturating_add(MAX_PAYLOAD_LEN as u64);
    if counted.unpacked > most {
        return Err(invalid(format!(
            "the replay unpacks past {MAX_REPLAY_INFLATION} times its size: a bomb"
        )));
    }
    if read == 0 && !buf.is_empty() && counted.read(&mut [0u8; 1])? > 0 {
        return Err(invalid(
            "bytes after the end of the replay's stream".to_owned(),
        ));
    }
    Ok(read)
}

impl Read for UnpackedReplay {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let State::Fresh(_) = self.state {
            let State::Fresh(file) = std::mem::replace(&mut self.state, State::Ended) else {
                unreachable!("the state was fresh");
            };
            self.state = Self::open(file)?;
        }
        let read = match &mut self.state {
            State::Fresh(_) => unreachable!("opened above"),
            State::Open(decoder) => read_open(decoder, buf),
            State::Ended => Ok(0),
        };
        if read
            .as_ref()
            .is_err_and(|error| error.kind() != ErrorKind::Interrupted)
        {
            self.state = State::Ended;
        }
        read
    }
}

fn invalid(why: String) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, why)
}

#[cfg(test)]
mod tests;
