//! A number that names the wire layout and the id registry in force.

/// FNV-1a over `registry/wire.txt` followed by `registry/ids.txt`.
///
/// The registries only grow, so two builds with the same fingerprint have
/// byte-identical registries and with them identical wire layouts and ids. A
/// client sends it in its [`ClientMsg::Hello`](crate::ClientMsg::Hello), and
/// the server refuses one that differs from its own with
/// [`ServerMsg::WireMismatch`](crate::ServerMsg::WireMismatch), so builds
/// that disagree fail at connection time instead of in mid-match.
pub const WIRE_FINGERPRINT: u64 = fnv1a(&[
    include_str!("../registry/wire.txt").as_bytes(),
    include_str!("../registry/ids.txt").as_bytes(),
]);

const fn fnv1a(parts: &[&[u8]]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut part = 0;
    while part < parts.len() {
        let bytes = parts[part];
        let mut i = 0;
        while i < bytes.len() {
            hash ^= bytes[i] as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
            i += 1;
        }
        part += 1;
    }
    hash
}
