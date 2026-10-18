//! A WebSocket message as a bota frame, for the carriers that read whole
//! messages (native `websocket`, browser `web`).

use std::io::ErrorKind;

use bota_proto::{LEN_PREFIX, MAX_PAYLOAD_LEN, ServerMsg, decode_payload};

/// The server message in one binary WebSocket message: the length prefix
/// must say exactly what follows it.
pub(crate) fn decode(bytes: &[u8]) -> std::io::Result<ServerMsg> {
    let invalid = |why: String| std::io::Error::new(ErrorKind::InvalidData, why);
    let (prefix, payload) = bytes
        .split_first_chunk::<LEN_PREFIX>()
        .ok_or_else(|| invalid("a WebSocket message shorter than a frame prefix".to_owned()))?;
    let len = u32::from_le_bytes(*prefix) as usize;
    if len != payload.len() || len > MAX_PAYLOAD_LEN {
        return Err(invalid(format!(
            "a frame prefix says {len} bytes but the message holds {}",
            payload.len()
        )));
    }
    decode_payload(payload).map_err(|error| {
        invalid(format!(
            "failed to decode server message: {error}; likely a protocol mismatch, build client \
and server from matching commits"
        ))
    })
}
