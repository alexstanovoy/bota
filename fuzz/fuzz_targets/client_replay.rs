//! A replay file played by a front end's app: framing, records and every
//! message the app folds in, frame after frame. No file panics the player.
#![no_main]

use bota_client_core::{App, ReplayPlayer, Source};
use libfuzzer_sys::fuzz_target;

/// Frames played: enough to drain an input of the fuzzer's size.
const FRAMES: usize = 256;

fuzz_target!(|data: &[u8]| {
    let player = ReplayPlayer::from_reader(std::io::Cursor::new(data.to_vec()));
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
