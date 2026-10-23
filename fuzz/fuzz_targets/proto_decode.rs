//! Every message either side decodes, the frame reader both clients use, and
//! the unpacker of a compressed link: no input panics or unpacks past its
//! bound, and whatever decodes encodes back to the same message.
#![no_main]

use bota_proto::{
    ClientMsg, FrameReader, LEN_PREFIX, ReplayRecord, ServerMsg, Unpacker, decode_payload,
    encode_frame_to_vec,
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

fuzz_target!(|data: &[u8]| {
    round_trip::<ClientMsg>(data);
    round_trip::<ServerMsg>(data);
    round_trip::<ReplayRecord>(data);
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
