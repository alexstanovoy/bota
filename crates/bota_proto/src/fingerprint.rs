//! Numbers that name a build's wire.

/// FNV-1a over `registry/wire.txt` followed by `registry/ids.txt`.
///
/// The registries only grow, so two builds with the same digest have
/// byte-identical registries and with them identical wire layouts and ids.
/// It is one half of the wire fingerprint a [`ClientMsg::Hello`] carries and
/// a [`ReplayHeader`] names; the other half is the version of the rules a
/// client checks its orders against, which this crate does not hold (see the
/// [`contract`](crate::contract)).
///
/// [`ClientMsg::Hello`]: crate::ClientMsg::Hello
/// [`ReplayHeader`]: crate::ReplayHeader
pub const REGISTRY_DIGEST: u64 = fnv1a(&[
    include_str!("../registry/wire.txt").as_bytes(),
    include_str!("../registry/ids.txt").as_bytes(),
]);

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
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
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
