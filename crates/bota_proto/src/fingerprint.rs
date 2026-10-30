//! Numbers that name a build's wire.

use crate::ModifierSpec;

/// FNV-1a over `registry/wire.txt`, then the value of every
/// [`limits`](crate::limits) bound and of the cheat modifier bounds
/// ([`ModifierSpec`](crate::ModifierSpec)), each as eight little endian bytes.
///
/// Two builds with the same digest have identical wire layouts and the same
/// idea of how much a message may hold. The ids of `registry/ids.txt` are
/// left out: they only grow, and a build that holds more of them still
/// meets one that holds fewer, on the items both hold
/// ([`ClientMsg::Catalogue`](crate::ClientMsg::Catalogue)). It is one half of the wire fingerprint a [`ClientMsg::Hello`] carries and
/// a [`ReplayHeader`] names; the other half is the version of the rules a
/// client checks its orders against, which this crate does not hold (see the
/// [`contract`](crate::contract)).
///
/// [`ClientMsg::Hello`]: crate::ClientMsg::Hello
/// [`ReplayHeader`]: crate::ReplayHeader
pub const REGISTRY_DIGEST: u64 = fnv1a_values(
    fnv1a_values(
        fnv1a(&[include_str!("../registry/wire.txt").as_bytes()]),
        crate::limits::VALUES,
    ),
    &[
        ModifierSpec::MAX_RESIST as u64,
        ModifierSpec::MAX_STATUS_RESIST as u64,
        ModifierSpec::MIN_SCALE as u64,
        ModifierSpec::MAX_SCALE as u64,
    ],
);

/// A fingerprint as 16 hex digits, the way `bota_server --wire-fingerprint`
/// prints it.
pub fn fingerprint_to_hex(fingerprint: u64) -> String {
    format!("{fingerprint:016x}")
}

/// A fingerprint from its hex digits.
pub fn fingerprint_from_hex(hex: &str) -> Option<u64> {
    u64::from_str_radix(hex, 16).ok()
}

/// FNV-1a over `parts` one after another, carriage returns left out: a text
/// checked out with Windows line ends hashes as it does with Unix ones.
pub const fn fnv1a(parts: &[&[u8]]) -> u64 {
    fnv1a_from(0xcbf2_9ce4_8422_2325_u64, parts)
}

/// `hash` continued over `values`, each as eight little endian bytes.
const fn fnv1a_values(hash: u64, values: &[u64]) -> u64 {
    let mut hash = hash;
    let mut at = 0;
    while at < values.len() {
        let bytes = values[at].to_le_bytes();
        hash = fnv1a_from(hash, &[&bytes]);
        at += 1;
    }
    hash
}

const fn fnv1a_from(mut hash: u64, parts: &[&[u8]]) -> u64 {
    let mut part = 0;
    while part < parts.len() {
        let bytes = parts[part];
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'\r' {
                hash ^= bytes[i] as u64;
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
            i += 1;
        }
        part += 1;
    }
    hash
}
