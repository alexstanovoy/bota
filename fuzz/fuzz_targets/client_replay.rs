//! A replay file played by a front end's app: framing, records and every
//! message the app folds in, frame after frame. No file panics the player.
//! The input follows this build's header, so the fuzzer spends itself on the
//! records; the header's own refusals are a short table of unit tests.
#![no_main]

use bota_client_core::{App, ReplayPlayer, Source};
use bota_proto::{ReplayHeader, ReplayRecord, encode_frame_to_vec};
use libfuzzer_sys::fuzz_target;

/// Frames played: enough to drain an input of the fuzzer's size.
const FRAMES: usize = 256;

fuzz_target!(|data: &[u8]| {
    let mut bytes = encode_frame_to_vec(&ReplayRecord::Header(ReplayHeader::CURRENT))
        .expect("a header encodes");
    bytes.extend_from_slice(data);
    let player = ReplayPlayer::from_reader(std::io::Cursor::new(bytes));
    let mut app = App::new(Source::Replay(Box::new(player)));
    for _ in 0..FRAMES {
        app.pump(0.25);
        app.cues.clear();
        if let Source::Replay(player) = &app.source
            && (player.finished() || player.error().is_some())
        {
            break;
        }
    }
});
