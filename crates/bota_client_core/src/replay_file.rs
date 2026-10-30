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

/// Most bytes of a trailer a replay may carry after its stream.
pub const MAX_TRAILER_BYTES: usize = 2 * 1024 * 1024;

impl Counted {
    /// Whether nothing follows the stream but, at most, skippable frames (RFC 8878: magic
    /// `0x184D2A50..=0x184D2A5F`, a length, that many bytes), which a decoder ignores. The
    /// metadata a replay carries is one (see `bota_replay::trailer`).
    fn only_a_trailer_follows(&mut self) -> io::Result<bool> {
        loop {
            let mut head = [0u8; 8];
            let got = read_up_to(&mut self.file, &mut head)?;
            if got == 0 {
                return Ok(true);
            }
            let magic = u32::from_le_bytes(head[..4].try_into().expect("four bytes"));
            if got < head.len() || magic & 0xFFFF_FFF0 != 0x184D_2A50 {
                return Ok(false);
            }
            let size = u64::from(u32::from_le_bytes(
                head[4..].try_into().expect("four bytes"),
            ));
            if size > MAX_TRAILER_BYTES as u64 {
                return Ok(false);
            }
            let skipped = io::copy(&mut (&mut self.file).take(size), &mut io::sink())?;
            if skipped < size {
                return Ok(false);
            }
        }
    }
}

/// Reads until `buf` is full or the reader ends; how many bytes it got.
fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match reader.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(got)
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
    if read == 0 && !buf.is_empty() && !counted.only_a_trailer_follows()? {
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
