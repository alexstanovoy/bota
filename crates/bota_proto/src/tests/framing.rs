//! Reassembling messages from a stream that arrives in arbitrary pieces.

use super::fixtures::*;
use crate::*;

fn wire(msgs: &[ClientMsg]) -> Vec<u8> {
    let mut out = Vec::new();
    for msg in msgs {
        encode_frame(msg, &mut out).expect("encode");
    }
    out
}

#[test]
fn nothing_is_yielded_before_a_whole_frame_arrives() {
    let mut reader = FrameReader::new();
    assert_eq!(reader.next_message::<ClientMsg>().unwrap(), None);

    let bytes = wire(&all_client_msgs()[..1]);
    reader.push(&bytes[..LEN_PREFIX]);
    assert_eq!(reader.next_message::<ClientMsg>().unwrap(), None);
}

#[test]
fn one_byte_at_a_time() {
    let msgs = all_client_msgs();
    let bytes = wire(&msgs);

    let mut reader = FrameReader::new();
    let mut got = Vec::new();
    for byte in &bytes {
        reader.push(std::slice::from_ref(byte));
        while let Some(msg) = reader.next_message::<ClientMsg>().unwrap() {
            got.push(msg);
        }
    }

    assert_eq!(got, msgs);
}

#[test]
fn many_frames_in_one_push() {
    let msgs = all_client_msgs();
    let bytes = wire(&msgs);

    let mut reader = FrameReader::new();
    reader.push(&bytes);

    let mut got = Vec::new();
    while let Some(msg) = reader.next_message::<ClientMsg>().unwrap() {
        got.push(msg);
    }

    assert_eq!(got, msgs);
}

#[test]
fn a_trailing_partial_frame_is_kept() {
    let msgs = all_client_msgs();
    let bytes = wire(&msgs);
    let cut = bytes.len() - 1;

    let mut reader = FrameReader::new();
    reader.push(&bytes[..cut]);

    let mut got = 0;
    while reader.next_message::<ClientMsg>().unwrap().is_some() {
        got += 1;
    }
    assert_eq!(got, msgs.len() - 1, "the last frame is incomplete");

    reader.push(&bytes[cut..]);
    assert_eq!(
        reader.next_message::<ClientMsg>().unwrap(),
        Some(msgs[msgs.len() - 1].clone()),
        "the partial frame's bytes were held, not dropped"
    );
}

#[test]
fn an_oversized_length_is_rejected_before_any_payload_arrives() {
    let claimed = MAX_PAYLOAD_LEN + 1;
    let mut bytes = (claimed as u32).to_le_bytes().to_vec();
    bytes.push(0);

    let mut reader = FrameReader::new();
    reader.push(&bytes);

    assert_eq!(
        reader.next_message::<ClientMsg>(),
        Err(CodecError::TooLarge { len: claimed }),
        "the reader must refuse on the prefix alone, not wait for the payload"
    );
}

#[test]
fn a_garbage_payload_is_malformed() {
    let payload = [0xffu8; 8];
    let mut bytes = (payload.len() as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(&payload);

    let mut reader = FrameReader::new();
    reader.push(&bytes);

    assert_eq!(
        reader.next_message::<ClientMsg>(),
        Err(CodecError::Malformed)
    );
}

#[test]
fn a_truncated_payload_is_malformed_not_a_wait() {
    let msg = ClientMsg::Hello {
        role: Role::Player,
        name: "a fairly long message body".to_string(),
        key: None,
        wire_fingerprint: REGISTRY_DIGEST,
    };
    let frame = encode_frame_to_vec(&msg).unwrap();
    let payload_len = frame.len() - LEN_PREFIX;

    let mut bytes = ((payload_len - 4) as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(&frame[LEN_PREFIX..LEN_PREFIX + payload_len - 4]);

    let mut reader = FrameReader::new();
    reader.push(&bytes);
    assert_eq!(
        reader.next_message::<ClientMsg>(),
        Err(CodecError::Malformed)
    );
}

#[test]
fn decoding_as_the_wrong_message_type_fails() {
    let frame = encode_frame_to_vec(&ServerMsg::ParticipantLeft {
        player_id: PlayerId(3),
        slot: Some(SlotId(0)),
    })
    .unwrap();

    let mut reader = FrameReader::new();
    reader.push(&frame);
    assert!(reader.next_message::<ClientMsg>().is_err());
}

#[test]
fn a_frame_is_a_prefix_plus_its_payload() {
    let msg = ClientMsg::Ack { tick: 5400 };
    let frame = encode_frame_to_vec(&msg).unwrap();

    let mut prefix = [0u8; LEN_PREFIX];
    prefix.copy_from_slice(&frame[..LEN_PREFIX]);
    let declared = u32::from_le_bytes(prefix) as usize;

    assert_eq!(declared, frame.len() - LEN_PREFIX);
    assert_eq!(
        decode_payload::<ClientMsg>(&frame[LEN_PREFIX..]).unwrap(),
        msg
    );
}

#[test]
fn frames_are_handed_out_whole_and_an_oversized_one_refused() {
    let mut reader = FrameReader::new();
    let small = encode_frame_to_vec(&1u16).unwrap();
    reader.push(&small);
    assert_eq!(reader.next_frame(), Ok(Some(small)));
    assert_eq!(reader.next_frame(), Ok(None));
    let claimed = MAX_PAYLOAD_LEN + 1;
    reader.push(&(claimed as u32).to_le_bytes());
    assert_eq!(
        reader.next_frame(),
        Err(CodecError::TooLarge { len: claimed })
    );
}

/// A part that unpacks past what the server ever flushes at once is a
/// bomb: the stream is refused once it has unpacked that much, whatever it
/// unpacks to, whole frames of pings included.
#[test]
fn a_compressed_part_unpacking_past_its_bound_is_refused() {
    let ping = encode_frame_to_vec(&ServerMsg::Ping).unwrap();
    let plain: Vec<u8> = ping
        .iter()
        .copied()
        .cycle()
        .take(MAX_UNPACKED_PART_LEN + 4 * ping.len())
        .collect();
    let part = miniz_oxide::deflate::compress_to_vec(&plain, 9);
    assert!(part.len() < plain.len() / 500, "{} bytes", part.len());
    let mut unpacker = Unpacker::new();
    assert_eq!(unpacker.take(ServerMsg::Compressed(part)), Ok(None));
    let mut pings = 0;
    let refused = loop {
        match unpacker.next_message() {
            Ok(Some(ServerMsg::Ping)) => pings += 1,
            Ok(other) => panic!("{other:?}"),
            Err(error) => break error,
        }
    };
    assert!(matches!(refused, CodecError::Unpacked { len } if len > MAX_UNPACKED_PART_LEN));
    assert!(pings * ping.len() <= MAX_UNPACKED_PART_LEN);
}
