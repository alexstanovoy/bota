//! A versioned envelope for the opaque match report carried by a launcher's RESULT line.

use crate::{
    CodecError, LEN_PREFIX, MAX_PAYLOAD_LEN, MatchReport, decode_payload, encode_frame_to_vec,
};

pub const MATCH_REPORT_MAGIC: [u8; 4] = *b"BMRP";
/// Raised whenever the report postcard payload layout changes.
pub const MATCH_REPORT_VERSION: u8 = 1;
const HEADER_LEN: usize = MATCH_REPORT_MAGIC.len() + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchReportError {
    MissingHeader,
    UnsupportedVersion(u8),
    Payload(CodecError),
}

impl std::fmt::Display for MatchReportError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingHeader => out.write_str("match report has no BMRP version header"),
            Self::UnsupportedVersion(version) => write!(
                out,
                "match report version {version} is not supported (expected {MATCH_REPORT_VERSION})"
            ),
            Self::Payload(error) => write!(out, "match report: {error}"),
        }
    }
}
impl std::error::Error for MatchReportError {}

/// Encodes a report with its magic and version, within the frame payload limit.
pub fn encode_match_report(report: &MatchReport) -> Result<Vec<u8>, CodecError> {
    let frame = encode_frame_to_vec(report)?;
    let payload = &frame[LEN_PREFIX..];
    let len = HEADER_LEN + payload.len();
    if len > MAX_PAYLOAD_LEN {
        return Err(CodecError::TooLarge { len });
    }
    let mut bytes = Vec::with_capacity(len);
    bytes.extend_from_slice(&MATCH_REPORT_MAGIC);
    bytes.push(MATCH_REPORT_VERSION);
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// Decodes this report version only. Unversioned reports and trailing bytes are refused.
pub fn decode_match_report(bytes: &[u8]) -> Result<MatchReport, MatchReportError> {
    if bytes.len() > MAX_PAYLOAD_LEN {
        return Err(MatchReportError::Payload(CodecError::TooLarge {
            len: bytes.len(),
        }));
    }
    let version = bytes
        .get(MATCH_REPORT_MAGIC.len())
        .copied()
        .filter(|_| bytes.starts_with(&MATCH_REPORT_MAGIC))
        .ok_or(MatchReportError::MissingHeader)?;
    if version != MATCH_REPORT_VERSION {
        return Err(MatchReportError::UnsupportedVersion(version));
    }
    decode_payload(&bytes[HEADER_LEN..]).map_err(MatchReportError::Payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MatchStats;
    fn report() -> MatchReport {
        MatchReport {
            picks: Vec::new(),
            stats: MatchStats {
                duration: 17,
                slots: Vec::new(),
                leaves: Vec::new(),
            },
        }
    }
    #[test]
    fn version_one_reports_roundtrip_and_reject_trailing_bytes() {
        let expected = report();
        let mut bytes = encode_match_report(&expected).expect("report encodes");
        assert!(bytes.starts_with(b"BMRP\x01"));
        assert_eq!(decode_match_report(&bytes), Ok(expected));
        bytes.push(0);
        assert_eq!(
            decode_match_report(&bytes),
            Err(MatchReportError::Payload(CodecError::Malformed))
        );
    }
    #[test]
    fn future_report_versions_fail_explicitly_before_decoding_the_body() {
        let error = decode_match_report(b"BMRP\x02").expect_err("unsupported version");
        assert_eq!(error, MatchReportError::UnsupportedVersion(2));
        assert_eq!(
            error.to_string(),
            "match report version 2 is not supported (expected 1)"
        );
    }
    #[test]
    fn unversioned_reports_have_no_compatibility_decode_path() {
        let raw = encode_frame_to_vec(&report()).expect("raw payload");
        assert_eq!(
            decode_match_report(&raw[LEN_PREFIX..]),
            Err(MatchReportError::MissingHeader)
        );
    }
    #[test]
    fn oversized_reports_are_rejected_before_decoding() {
        let bytes = vec![0; MAX_PAYLOAD_LEN + 1];
        assert_eq!(
            decode_match_report(&bytes),
            Err(MatchReportError::Payload(CodecError::TooLarge {
                len: bytes.len()
            }))
        );
    }
}
