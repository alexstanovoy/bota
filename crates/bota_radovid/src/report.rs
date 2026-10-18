//! The report of a finished match.

use bota_proto::{MatchReport, decode_payload};
use radovid_client::proto::MatchResult;

/// The picks and final numbers the game server reported, or none when the
/// match ended without a report (abandoned, crashed, stopped) or the bytes are
/// not bota's.
pub fn decode_report(result: &MatchResult) -> Option<MatchReport> {
    if result.game_report.is_empty() {
        return None;
    }
    decode_payload(&result.game_report).ok()
}
