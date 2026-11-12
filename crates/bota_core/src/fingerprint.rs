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
/// a server refuses a client of another one, neither its own nor one of
/// [`ADMITTED_WIRES`], with `ServerMsg::WireMismatch` (see the `bota_proto`
/// contract). The items are not in it: builds meet on
/// the items both hold ([`Catalogue`](crate::Catalogue)), so an item added
/// leaves older clients and replays playing.
pub const WIRE_FINGERPRINT: u64 = wire_fingerprint(REGISTRY_DIGEST, RULES_IDENTITY);

/// The wires of earlier builds this one plays with as their own builds did: a client, a server or
/// a replay of one of them is admitted, and the match meets on the items both hold
/// ([`Catalogue`](crate::Catalogue)), as with a build of this wire.
///
/// An entry names the wire of the build a change was made on, and only a change that leaves
/// every rule of that build's items, heroes and map as it was may add it: one that adds items
/// and the mechanics only they reach. The pinned determinism runs (`bota_server`'s
/// `determinism` pins, which play the oldest catalogue) holding unchanged across the change are
/// its evidence; a change that moves them changes the rules, and starts the list afresh.
pub const ADMITTED_WIRES: &[u64] = &[
    // 7ce36d97 (Dota 7.41f item batch 1 was made on it).
    0xb151_984c_e81b_91a4,
];

/// Whether a peer or a replay of `wire` plays with this build: its own wire or an admitted one.
pub const fn admits_wire(wire: u64) -> bool {
    wire == WIRE_FINGERPRINT || admits_wire_listed(wire)
}

const _: () = assert!(
    !admits_wire_listed(WIRE_FINGERPRINT),
    "this build's own wire is not an earlier one"
);

/// Whether `wire` is among [`ADMITTED_WIRES`].
const fn admits_wire_listed(wire: u64) -> bool {
    let mut at = 0;
    while at < ADMITTED_WIRES.len() {
        if ADMITTED_WIRES[at] == wire {
            return true;
        }
        at += 1;
    }
    false
}

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
