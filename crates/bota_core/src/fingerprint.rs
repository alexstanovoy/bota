//! The content identities of a build's rules and wire.

use bota_proto::REGISTRY_DIGEST;

/// A content identity, not an ordered release number: the first eight SHA-256
/// bytes, little endian, of the canonical production sources of the rule,
/// protocol and server crates, their maps and included data.
///
/// Computed at build time; comments, layout, documentation and test-only items
/// do not count. Item rows use [`Catalogue`](crate::Catalogue) prefix digests,
/// so appending data for an item preserves agreement on older catalogues.
/// See `DEVELOPMENT.md` for the source boundary and canonicalization contract.
pub const RULES_IDENTITY: u64 = include!(concat!(env!("OUT_DIR"), "/rules_identity.rs"));

/// The wire fingerprint: the [`REGISTRY_DIGEST`] and the [`RULES_IDENTITY`]
/// together. A client's `Hello` carries it, a replay's header names it, and
/// a server refuses a client of another one with `ServerMsg::WireMismatch`
/// (see the `bota_proto` contract). The items are not in it: builds meet on
/// the items both hold ([`Catalogue`](crate::Catalogue)), so an item added
/// leaves older clients and replays playing.
pub const WIRE_FINGERPRINT: u64 = wire_fingerprint(REGISTRY_DIGEST, RULES_IDENTITY);

// Unlike text hashing, binary hashing must retain every byte, including 0x0d.
const fn wire_fingerprint(registry: u64, rules: u64) -> u64 {
    let parts = [registry.to_le_bytes(), rules.to_le_bytes()];
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut part = 0;
    while part < parts.len() {
        let mut byte = 0;
        while byte < parts[part].len() {
            hash ^= parts[part][byte] as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
            byte += 1;
        }
        part += 1;
    }
    hash
}

#[cfg(test)]
mod tests {
    #[test]
    fn carriage_return_bytes_distinguish_binary_rule_identities() {
        assert_ne!(
            super::wire_fingerprint(1, 0x0d01),
            super::wire_fingerprint(1, 0x010d)
        );
    }
}
