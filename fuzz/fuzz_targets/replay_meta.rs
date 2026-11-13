//! A replay's metadata file from a stranger: the first byte picks the file
//! read as it stands, the one a replay carries in its trailer, or the one a
//! player writes again from a replay's records. No file panics a reader or
//! makes it hold more than its bounds, and what the recorder writes reads
//! back.
#![no_main]

use std::io::Cursor;

use bota_client_core::{meta_in_replay, meta_of_replay};
use bota_proto::{ReplayHeader, ReplayRecord, encode_frame_to_vec};
use bota_replay::limits::{END_RESERVE_BYTES, MAX_META_BYTES};
use bota_replay::meta::{MetaRecord, decode, with_profiles};
use libfuzzer_sys::fuzz_target;

/// Most bytes a metadata file may hold, its reserve for the end included.
const MAX_FILE: usize = MAX_META_BYTES + END_RESERVE_BYTES;

fuzz_target!(|data: &[u8]| {
    let Some((&mode, data)) = data.split_first() else {
        return;
    };
    match mode % 3 {
        0 => {
            check_profiles(data);
        }
        1 => {
            if let Ok(Some(meta)) = meta_in_replay(&mut Cursor::new(data)) {
                assert!(meta.len() <= MAX_FILE, "a trailer past its bound");
                check_profiles(&meta);
            }
        }
        _ => {
            let mut replay = encode_frame_to_vec(&ReplayRecord::Header(ReplayHeader::new(
                bota_core::WIRE_FINGERPRINT,
            )))
            .expect("a header encodes");
            replay.extend_from_slice(data);
            if let Ok(meta) = meta_of_replay(Cursor::new(replay)) {
                assert!(meta.len() <= MAX_FILE, "a written file past its bound");
                if let Err(error) = decode(&meta) {
                    panic!("the recorder wrote a file it cannot read: {error}");
                }
            }
        }
    }
});

fn check_profiles(bytes: &[u8]) {
    if let Ok(meta) = decode(bytes) {
        let profiles = meta.profiles().unwrap_or_default();
        if let Some(enriched) = with_profiles(bytes, &profiles) {
            assert!(enriched.len() <= MAX_FILE);
            let rewritten = decode(&enriched).expect("enriched metadata reads");
            assert_eq!(rewritten.profiles(), Some(profiles));
            let facts = |meta: bota_replay::meta::Meta| {
                meta.records
                    .into_iter()
                    .filter(|record| !matches!(record, MetaRecord::Profiles { .. }))
                    .collect::<Vec<_>>()
            };
            assert_eq!(facts(meta), facts(rewritten));
        }
    }
}
