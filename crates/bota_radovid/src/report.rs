//! The report of a finished match.

use bota_proto::{MatchReport, MatchReportError, decode_match_report};
use radovid_client::proto::MatchResult;

/// The picks and final numbers the game server reported. No report is distinct from
/// a present report that is malformed or requires a different format version.
pub fn decode_report(result: &MatchResult) -> Result<Option<MatchReport>, MatchReportError> {
    if result.game_report.is_empty() {
        return Ok(None);
    }
    decode_match_report(&result.game_report).map(Some)
}
