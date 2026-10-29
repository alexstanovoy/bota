//! Unpacking a connection's compressed stream: what
//! [`ServerMsg::Compressed`] parts carry.

use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use crate::{CodecError, FrameReader, LEN_PREFIX, MAX_PAYLOAD_LEN, ServerMsg};

/// Most bytes one [`ServerMsg::Compressed`] part may unpack to: what the
/// server flushes at once is one message or one tick, a snapshot and its
/// events, each a frame of at most [`MAX_PAYLOAD_LEN`]. A part that unpacks
/// to more is a bomb, and the stream is refused.
pub const MAX_UNPACKED_PART_LEN: usize = 2 * (LEN_PREFIX + MAX_PAYLOAD_LEN);

/// Bytes unpacked at a time: what is unpacked waits only while no whole
/// frame is, so a part never takes more memory than a frame and this.
const STEP: usize = 16 * 1024;

/// The server messages of one connection, compressed parts unpacked in the
/// order they came.
///
/// Hand it each message off the wire with [`Unpacker::take`] once
/// [`Unpacker::next_message`] has none left: a plain message comes back at
/// once, a part's messages come out of `next_message` one by one, unpacked
/// as they are asked for.
#[derive(Default)]
pub struct Unpacker {
    state: Option<Box<InflateState>>,
    /// Where the inflater writes, allocated with the state and reused.
    step: Vec<u8>,
    /// The part being unpacked, and how much of it has been.
    part: Vec<u8>,
    used: usize,
    /// Bytes the part unpacked to so far.
    unpacked: usize,
    /// Whether the inflater may still hold unpacked bytes it had no room
    /// for.
    pending: bool,
    frames: FrameReader,
}

impl Unpacker {
    /// An unpacker that has seen no part yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes in the next message off the wire: a plain one is handed back,
    /// a [`ServerMsg::Compressed`] part is kept for
    /// [`Unpacker::next_message`]. A plain message must not stand inside a
    /// frame the parts before it began, and a part must not come while the
    /// one before has messages left.
    pub fn take(&mut self, msg: ServerMsg) -> Result<Option<ServerMsg>, CodecError> {
        if self.used < self.part.len() || self.pending {
            return Err(CodecError::Malformed);
        }
        let ServerMsg::Compressed(part) = msg else {
            return match self.frames.is_empty() {
                true => Ok(Some(msg)),
                false => Err(CodecError::Malformed),
            };
        };
        if self.state.is_none() {
            self.state = Some(InflateState::new_boxed(DataFormat::Raw));
            self.step = vec![0; STEP];
        }
        self.part = part;
        self.used = 0;
        self.unpacked = 0;
        self.pending = true;
        Ok(None)
    }

    /// The next message the parts taken in hold, unpacking as much as it
    /// takes; none when they hold no whole message more.
    pub fn next_message(&mut self) -> Result<Option<ServerMsg>, CodecError> {
        loop {
            match self.frames.next_message::<ServerMsg>()? {
                Some(ServerMsg::Compressed(_)) => return Err(CodecError::Malformed),
                Some(msg) => return Ok(Some(msg)),
                None if !self.pending => return Ok(None),
                None => {}
            }
            let state = self.state.as_mut().ok_or(CodecError::Malformed)?;
            let result = inflate(
                state,
                &self.part[self.used..],
                &mut self.step,
                MZFlush::None,
            );
            self.used += result.bytes_consumed;
            self.unpacked += result.bytes_written;
            if self.unpacked > MAX_UNPACKED_PART_LEN {
                return Err(CodecError::Unpacked { len: self.unpacked });
            }
            self.frames.push(&self.step[..result.bytes_written]);
            let stuck = result.bytes_written == 0 && result.bytes_consumed == 0;
            self.pending = self.used < self.part.len() || result.bytes_written == STEP;
            // The server's stream never ends, and it flushes at the end of
            // every part: a part that stops making progress before its end
            // is broken.
            match result.status {
                Ok(MZStatus::Ok) | Err(MZError::Buf) => {}
                Ok(_) | Err(_) => return Err(CodecError::Malformed),
            }
            if stuck {
                if self.used < self.part.len() {
                    return Err(CodecError::Malformed);
                }
                self.pending = false;
            }
        }
    }
}
