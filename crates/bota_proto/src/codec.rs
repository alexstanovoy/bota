//! Turning messages into bytes and back.
//!
//! A message on the wire is a little endian `u32` length followed by that many
//! bytes of postcard payload. Nothing here performs input or output.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::MAX_PAYLOAD_LEN;

/// Bytes in the length prefix.
pub const LEN_PREFIX: usize = 4;

/// Something went wrong turning bytes into a message or back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// A frame declared or would produce a payload above [`MAX_PAYLOAD_LEN`].
    TooLarge {
        /// The length that was rejected.
        len: usize,
    },
    /// The payload did not decode into the expected message, or a
    /// compressed stream was broken.
    Malformed,
    /// One compressed part unpacked to more than
    /// [`MAX_UNPACKED_PART_LEN`](crate::MAX_UNPACKED_PART_LEN): a bomb.
    Unpacked {
        /// The bytes it had unpacked to when it was refused.
        len: usize,
    },
}

impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CodecError::TooLarge { len } => {
                write!(f, "frame payload of {len} bytes exceeds {MAX_PAYLOAD_LEN}")
            }
            CodecError::Malformed => write!(f, "frame payload did not decode"),
            CodecError::Unpacked { len } => write!(
                f,
                "a compressed part unpacked to {len} bytes, over {}",
                crate::MAX_UNPACKED_PART_LEN
            ),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<postcard::Error> for CodecError {
    fn from(_: postcard::Error) -> Self {
        CodecError::Malformed
    }
}

/// Encode one message and append the whole frame to `out`.
pub fn encode_frame<T: Serialize + ?Sized>(msg: &T, out: &mut Vec<u8>) -> Result<(), CodecError> {
    let payload = postcard::to_allocvec(msg)?;
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(CodecError::TooLarge { len: payload.len() });
    }
    out.reserve(LEN_PREFIX + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(())
}

/// Encode one message into a frame of its own.
pub fn encode_frame_to_vec<T: Serialize + ?Sized>(msg: &T) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    encode_frame(msg, &mut out)?;
    Ok(out)
}

/// The length of a message's payload, without encoding it.
pub fn payload_len<T: Serialize + ?Sized>(msg: &T) -> Result<usize, CodecError> {
    Ok(postcard::serialize_with_flavor(
        msg,
        postcard::ser_flavors::Size::default(),
    )?)
}

/// Decode one message from a frame payload, with the length prefix removed.
///
/// The payload is the message and nothing else: bytes after it are refused, so
/// a message has one encoding. A collection past the bound its field names
/// (see [`limits`](crate::limits)) is refused before it is read.
pub fn decode_payload<T: DeserializeOwned>(payload: &[u8]) -> Result<T, CodecError> {
    let (message, rest) = postcard::take_from_bytes(payload)?;
    if rest.is_empty() {
        Ok(message)
    } else {
        Err(CodecError::Malformed)
    }
}

/// Reassembles messages from a byte stream that arrives in arbitrary pieces.
///
/// ```
/// # use bota_proto::{encode_frame, FrameReader};
/// let mut wire = Vec::new();
/// encode_frame(&7u16, &mut wire).unwrap();
///
/// let mut reader = FrameReader::new();
/// reader.push(&wire[..1]);
/// assert_eq!(reader.next_message::<u16>().unwrap(), None);
/// reader.push(&wire[1..]);
/// assert_eq!(reader.next_message::<u16>().unwrap(), Some(7));
/// ```
#[derive(Clone, Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    /// Where the frame at the front starts: the bytes before it were taken.
    /// Taking a frame only moves this, so a push of many small frames costs
    /// what its bytes do.
    start: usize,
}

impl FrameReader {
    /// A reader holding no bytes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes that just arrived.
    pub fn push(&mut self, bytes: &[u8]) {
        self.compact();
        self.buf.extend_from_slice(bytes);
    }

    /// Drops the bytes taken, once they are at least as many as the bytes
    /// kept: each byte is moved at most once for every byte taken before
    /// it, so a hostile stream cannot make the moving quadratic, and the
    /// buffer holds at most twice what waits.
    fn compact(&mut self) {
        if self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start >= self.buf.len() - self.start {
            self.buf.drain(..self.start);
            self.start = 0;
        }
    }

    /// Take the next complete message, or `None` if one has not fully arrived.
    ///
    /// After a [`CodecError`] the offending bytes stay at the front, and every
    /// later call returns the same error.
    pub fn next_message<T: DeserializeOwned>(&mut self) -> Result<Option<T>, CodecError> {
        let Some(end) = self.frame_end()? else {
            return Ok(None);
        };
        let msg = decode_payload(&self.buf[self.start + LEN_PREFIX..self.start + end])?;
        self.start += end;
        Ok(Some(msg))
    }

    /// Whether no byte waits: every byte pushed belonged to a frame taken.
    pub fn is_empty(&self) -> bool {
        self.start == self.buf.len()
    }

    /// Take the next complete frame whole, length prefix included and payload
    /// undecoded, or `None` if one has not fully arrived. A bridge copies
    /// frames this way without reading them.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, CodecError> {
        let Some(end) = self.frame_end()? else {
            return Ok(None);
        };
        let frame = self.buf[self.start..self.start + end].to_vec();
        self.start += end;
        Ok(Some(frame))
    }

    /// Where the frame at the front ends, once all of it has arrived. Its
    /// length is checked as soon as the prefix is in.
    fn frame_end(&self) -> Result<Option<usize>, CodecError> {
        let Some(prefix) = self.buf[self.start..].first_chunk::<LEN_PREFIX>() else {
            return Ok(None);
        };
        let len = u32::from_le_bytes(*prefix) as usize;
        if len > MAX_PAYLOAD_LEN {
            return Err(CodecError::TooLarge { len });
        }
        let end = LEN_PREFIX + len;
        Ok((self.buf.len() - self.start >= end).then_some(end))
    }
}
