//! Turning messages into bytes and back.
//!
//! A message on the wire is a little endian `u32` length followed by that many
//! bytes of postcard payload. Nothing here performs input or output.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Largest payload accepted in one frame, in bytes.
pub const MAX_PAYLOAD_LEN: usize = 4 * 1024 * 1024;

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
    /// The payload did not decode into the expected message.
    Malformed,
}

impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CodecError::TooLarge { len } => {
                write!(f, "frame payload of {len} bytes exceeds {MAX_PAYLOAD_LEN}")
            }
            CodecError::Malformed => write!(f, "frame payload did not decode"),
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

/// Decode one message from a frame payload, with the length prefix removed.
pub fn decode_payload<T: DeserializeOwned>(payload: &[u8]) -> Result<T, CodecError> {
    Ok(postcard::from_bytes(payload)?)
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
}

impl FrameReader {
    /// A reader holding no bytes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes that just arrived.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// How many bytes are held but not yet part of a complete frame.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// Take the next complete message, or `None` if one has not fully arrived.
    ///
    /// After a [`CodecError`] the offending bytes stay at the front, and every
    /// later call returns the same error.
    pub fn next_message<T: DeserializeOwned>(&mut self) -> Result<Option<T>, CodecError> {
        let Some(len) = self.peek_len()? else {
            return Ok(None);
        };
        let end = LEN_PREFIX + len;
        if self.buf.len() < end {
            return Ok(None);
        }
        let msg = decode_payload(&self.buf[LEN_PREFIX..end])?;
        self.buf.drain(..end);
        Ok(Some(msg))
    }

    /// Length of the frame at the front, once its prefix has arrived.
    fn peek_len(&self) -> Result<Option<usize>, CodecError> {
        if self.buf.len() < LEN_PREFIX {
            return Ok(None);
        }
        let mut prefix = [0u8; LEN_PREFIX];
        prefix.copy_from_slice(&self.buf[..LEN_PREFIX]);
        let len = u32::from_le_bytes(prefix) as usize;
        if len > MAX_PAYLOAD_LEN {
            return Err(CodecError::TooLarge { len });
        }
        Ok(Some(len))
    }
}
