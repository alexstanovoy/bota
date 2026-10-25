//! The number a build's wire is known by: its registries and its rules.

use bota_proto::{REGISTRY_DIGEST, fnv1a};

/// The version of the rules a client checks its orders against and predicts
/// with: every table and integer function of this crate.
///
/// Bump it with every change to what they decide: a number, a table entry,
/// what a function returns. A test pins it together with a digest of this
/// crate's code that ignores comments and layout, and fails on any change of
/// that code until the pin is renewed; a changed number or string forces the
/// bump. A comment, a reformatting or a refactor that decides nothing
/// differently renews the pin and leaves the version, so builds that agree on
/// the rules keep meeting.
pub const RULES_VERSION: u32 = 51;

/// The wire fingerprint: the [`REGISTRY_DIGEST`] and the [`RULES_VERSION`]
/// together. A client's `Hello` carries it, a replay's header names it, and
/// a server refuses a client of another one with `ServerMsg::WireMismatch`
/// (see the `bota_proto` contract).
pub const WIRE_FINGERPRINT: u64 =
    fnv1a(&[&REGISTRY_DIGEST.to_le_bytes(), &RULES_VERSION.to_le_bytes()]);
