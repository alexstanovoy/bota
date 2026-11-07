//! Every message either side decodes, the frame reader both clients use, and
//! the unpacker of a compressed link: no input panics or unpacks past its
//! bound, and whatever decodes encodes back to the same message.
#![no_main]

use bota_proto::{
    ClientMsg, FrameReader, LEN_PREFIX, MATCH_REPORT_MAGIC, MATCH_REPORT_VERSION, MAX_PAYLOAD_LEN,
    ReplayRecord, ServerMsg, Unpacker, decode_match_report, decode_payload, encode_frame_to_vec,
    encode_match_report,
};
use libfuzzer_sys::fuzz_target;

/// Whatever decodes encodes, and decodes again to what encodes the same
/// (bytes, not values: a float may be NaN).
fn round_trip<T>(data: &[u8])
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    if let Ok(msg) = decode_payload::<T>(data) {
        let frame = encode_frame_to_vec(&msg).expect("a decoded message encodes");
        let again = decode_payload::<T>(&frame[LEN_PREFIX..]).expect("and decodes");
        assert_eq!(frame, encode_frame_to_vec(&again).expect("and encodes"));
    }
}

fn round_trip_report(data: &[u8]) {
    if let Ok(report) = decode_match_report(data) {
        let encoded = encode_match_report(&report).expect("a decoded report encodes");
        let again = decode_match_report(&encoded).expect("and decodes");
        assert_eq!(encoded, encode_match_report(&again).expect("and encodes"));
    }
}

fuzz_target!(|data: &[u8]| {
    round_trip::<ClientMsg>(data);
    round_trip::<ServerMsg>(data);
    round_trip::<ReplayRecord>(data);
    round_trip_report(data);
    // Exercise arbitrary report bodies without requiring mutations to discover the magic.
    if let Some(size) = data
        .len()
        .checked_add(MATCH_REPORT_MAGIC.len() + 1)
        .filter(|size| *size <= MAX_PAYLOAD_LEN)
    {
        let mut versioned = Vec::with_capacity(size);
        versioned.extend_from_slice(&MATCH_REPORT_MAGIC);
        versioned.push(MATCH_REPORT_VERSION);
        versioned.extend_from_slice(data);
        round_trip_report(&versioned);
    }
    let mut reader = FrameReader::new();
    for piece in bota_fuzz::frames(data) {
        reader.push(piece);
        while let Ok(Some(_)) = reader.next_message::<ServerMsg>() {}
    }
    // Every piece a compressed part, until the stream breaks.
    let mut unpacker = Unpacker::new();
    for piece in bota_fuzz::frames(data) {
        if unpacker
            .take(ServerMsg::Compressed(piece.to_vec()))
            .is_err()
        {
            break;
        }
        let unpacked = std::iter::from_fn(|| unpacker.next_message().ok().flatten()).count();
        assert!(unpacked <= bota_proto::MAX_UNPACKED_PART_LEN / LEN_PREFIX);
    }
});
