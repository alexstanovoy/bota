//! A packed replay is untrusted: what it may make a player hold and do is
//! bounded, and a stream that asks for more is refused up front.

use std::io::{ErrorKind, Read};

use ruzstd::encoding::{CompressionLevel, compress_to_vec};

use super::{MAX_REPLAY_INFLATION, MAX_TRAILER_BYTES, UnpackedReplay, ZSTD_MAGIC};

fn unpack(packed: Vec<u8>) -> std::io::Result<Vec<u8>> {
    let mut unpacked = Vec::new();
    UnpackedReplay::new(std::io::Cursor::new(packed)).read_to_end(&mut unpacked)?;
    Ok(unpacked)
}

#[test]
fn a_stream_that_unpacks_far_past_its_size_is_refused_as_a_bomb() {
    // Sixteen MiB of one byte pack into a few hundred bytes.
    let packed = compress_to_vec(&vec![7u8; 16 << 20][..], CompressionLevel::Fastest);
    assert!((packed.len() as u64) * MAX_REPLAY_INFLATION < 16 << 20);
    let refused = unpack(packed).expect_err("a bomb");
    assert_eq!(refused.kind(), ErrorKind::InvalidData);
    assert!(refused.to_string().contains("a bomb"), "{refused}");
    // An honest stream of the same bytes as a replay holds them unpacks.
    let varied: Vec<u8> = (0..1u32 << 20).flat_map(u32::to_le_bytes).collect();
    let packed = compress_to_vec(&varied[..], CompressionLevel::Fastest);
    assert_eq!(unpack(packed).expect("an honest stream"), varied);
}

#[test]
fn a_stream_asking_for_a_window_above_the_bound_is_refused_before_any_byte() {
    // A frame header: no single segment, a window of 2^(10 + 20) bytes.
    let mut packed = ZSTD_MAGIC.to_vec();
    packed.extend([0x00, 20 << 3]);
    packed.extend([0u8; 64]);
    let refused = unpack(packed).expect_err("a window of a gibibyte");
    assert_eq!(refused.kind(), ErrorKind::InvalidData);
    assert!(refused.to_string().contains("does not open"), "{refused}");
}

#[test]
fn bytes_after_the_stream_are_refused() {
    let mut packed = compress_to_vec(&b"a whole stream"[..], CompressionLevel::Fastest);
    packed.push(0);
    let refused = unpack(packed).expect_err("a trailing byte");
    assert!(refused.to_string().contains("after the end"), "{refused}");
}

/// A skippable frame after the stream (the trailer a replay carries its metadata in) is skipped;
/// anything else after it is refused.
#[test]
fn a_skippable_frame_after_the_stream_is_skipped_and_other_bytes_are_not() {
    let stream = compress_to_vec(&b"a whole stream"[..], CompressionLevel::Fastest);
    let skippable = |magic: u32, payload: &[u8]| {
        let mut frame = magic.to_le_bytes().to_vec();
        frame.extend((payload.len() as u32).to_le_bytes());
        frame.extend(payload);
        frame
    };
    let with = |tail: Vec<u8>| [stream.clone(), tail].concat();
    assert_eq!(
        unpack(with(skippable(0x184D_2A50, &[1, 2, 3]))).expect("a trailer"),
        b"a whole stream"
    );
    assert_eq!(
        unpack(with(skippable(0x184D_2A5F, &[]))).expect("an empty one"),
        b"a whole stream"
    );
    for refused in [
        skippable(0x184D_2A60, &[1]),
        skippable(0x184D_2A50, &[1])[..7].to_vec(),
        skippable(0x184D_2A50, &[1, 2])[..9].to_vec(),
        [skippable(0x184D_2A50, &[1]), vec![0]].concat(),
        skippable(0x184D_2A50, &vec![0; MAX_TRAILER_BYTES + 1]),
    ] {
        let error = unpack(with(refused)).expect_err("not a trailer");
        assert!(error.to_string().contains("after the end"), "{error}");
    }
}
