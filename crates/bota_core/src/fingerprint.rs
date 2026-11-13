//! The content identities of a build's rules and wire.

use bota_proto::{CataloguePrefix, REGISTRY_DIGEST};

use crate::Catalogue;

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

/// An earlier build this one plays with as its own build did: a client, a server or a replay of
/// it is admitted, and the match meets on the items both hold ([`Catalogue`](crate::Catalogue)),
/// as with a build of this wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmittedWire {
    /// Its wire fingerprint.
    pub wire: u64,
    /// The items it holds, which this build holds alike (a build-time check): what a client of
    /// this build says it holds when it speaks that wire.
    pub catalogue: CataloguePrefix,
}

impl AdmittedWire {
    /// Its items, as this build holds them.
    pub const fn items(self) -> Catalogue {
        match Catalogue::first(self.catalogue.items) {
            Some(items) => items,
            None => panic!("an admitted build holds no items this build does not"),
        }
    }
}

/// The wires of earlier builds this one plays with as their own builds did.
///
/// Only a change that leaves every rule of an earlier build's items, heroes and maps as it was
/// may admit that build's wire: one that adds items and the mechanics only they reach. The
/// evidence is the wire's corpus, `crates/bota_server/tests/admitted/WIRE/` (the wire in 16 hex
/// digits): scripted matches that buy and use every item of its catalogue, written by a build of
/// that wire (`tools/admitted.sh corpus COMMIT`), which this build plays again to the same
/// records, snapshots and world fingerprints (`bota_server`'s `admitted` test). A wire without a
/// corpus, or whose corpus this build plays otherwise, fails that test: a change that moves it
/// changes the rules, and takes the entry out. See `DEVELOPMENT.md`, "Admitted wires".
pub const ADMITTED_WIRES: &[AdmittedWire] = &[AdmittedWire {
    wire: 0x4df9_74f2_6e20_47eb,
    catalogue: CataloguePrefix {
        items: 94,
        digest: 12_760_854_504_706_889_410,
    },
}];

/// The earlier build of `wire`, when this build admits it.
pub const fn admitted_wire(wire: u64) -> Option<AdmittedWire> {
    let mut at = 0;
    while at < ADMITTED_WIRES.len() {
        if ADMITTED_WIRES[at].wire == wire {
            return Some(ADMITTED_WIRES[at]);
        }
        at += 1;
    }
    None
}

/// Whether a peer or a replay of `wire` plays with this build: its own wire or an admitted one.
pub const fn admits_wire(wire: u64) -> bool {
    wire == WIRE_FINGERPRINT || admitted_wire(wire).is_some()
}

/// The items a client of this build says it holds when it speaks `wire`: all of them on its own
/// wire, an admitted build's on that one's. None for a wire it does not admit.
pub const fn spoken_items(wire: u64) -> Option<Catalogue> {
    if wire == WIRE_FINGERPRINT {
        return Some(Catalogue::FULL);
    }
    match admitted_wire(wire) {
        Some(admitted) => Some(admitted.items()),
        None => None,
    }
}

const _: () = {
    assert!(
        admitted_wire(WIRE_FINGERPRINT).is_none(),
        "this build's own wire is not an earlier one"
    );
    let mut at = 0;
    while at < ADMITTED_WIRES.len() {
        let admitted = ADMITTED_WIRES[at];
        let held = admitted.items().prefix();
        assert!(
            held.items == admitted.catalogue.items && held.digest == admitted.catalogue.digest,
            "an admitted build's items are this build's first ones, unchanged"
        );
        let mut other = at + 1;
        while other < ADMITTED_WIRES.len() {
            assert!(
                ADMITTED_WIRES[other].wire != admitted.wire,
                "a wire is admitted once"
            );
            other += 1;
        }
        at += 1;
    }
};

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
