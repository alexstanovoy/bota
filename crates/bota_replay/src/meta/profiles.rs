//! Public account locations are untrusted claims, never connection instructions.
use super::{Meta, MetaRecord};
use serde::{
    Deserializer,
    de::{self, SeqAccess, Visitor},
};
use std::fmt;

pub const MAX_MATCHMAKER_BYTES: usize = 512;
pub const MAX_PROFILE_SEATS: usize = 10;

/// A validated snapshot. Absence of this record means unknown; a missing maker means local.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Profiles {
    matchmaker: Option<String>,
    seats: Vec<(u8, Option<u64>)>,
}
impl Profiles {
    pub fn new(matchmaker: Option<String>, seats: Vec<(u8, Option<u64>)>) -> Option<Self> {
        valid(matchmaker.as_deref(), &seats).then_some(Self { matchmaker, seats })
    }
    pub fn matchmaker(&self) -> Option<&str> {
        self.matchmaker.as_deref()
    }
    pub fn seats(&self) -> &[(u8, Option<u64>)] {
        &self.seats
    }
    pub fn account(&self, slot: u8) -> Option<u64> {
        self.seats
            .iter()
            .find(|(seat, _)| *seat == slot)
            .and_then(|(_, account)| *account)
    }
    pub fn record(&self) -> MetaRecord {
        MetaRecord::Profiles {
            matchmaker: self.matchmaker.clone(),
            seats: self.seats.clone(),
        }
    }
}
impl Meta {
    /// Malformed known records and broken framing make provenance ambiguous; future kinds do not.
    pub fn profiles(&self) -> Option<Profiles> {
        if self.damaged {
            return None;
        }
        let mut found = None;
        for record in &self.records {
            if let MetaRecord::Profiles { matchmaker, seats } = record {
                if found.is_some() {
                    return None;
                }
                found = Some(Profiles::new(matchmaker.clone(), seats.clone())?);
            }
        }
        found
    }
}

/// ASCII public URI, kept as text. No resolution, normalization, redirects or credentials.
pub fn public_matchmaker(value: &str) -> bool {
    if value.len() > MAX_MATCHMAKER_BYTES
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
        || value.contains(['@', '?', '#', '\\'])
    {
        return false;
    }
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "ws" | "wss" | "http" | "https") {
        return false;
    }
    let authority = rest.split('/').next().unwrap_or_default();
    !authority.is_empty()
        && authority
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-:[]".contains(&byte))
}
/// Whether a syntactically valid address can identify a service in a shared file.
/// This never resolves DNS; local literals and local-only names carry no shareable identity.
pub fn shareable_matchmaker(value: &str) -> bool {
    use std::net::IpAddr;
    if !public_matchmaker(value) {
        return false;
    }
    let rest = value
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let authority = rest.split('/').next().unwrap_or_default();
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split_once(']')
            .map(|(host, _)| host)
            .unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => public_ipv4(ip),
        Ok(IpAddr::V6(ip)) => ip.to_ipv4_mapped().map_or_else(
            || {
                !ip.is_loopback()
                    && !ip.is_unspecified()
                    && !ip.is_unique_local()
                    && !ip.is_unicast_link_local()
                    && !ip.is_multicast()
            },
            public_ipv4,
        ),
        Err(_) => public_hostname(host),
    }
}

fn public_hostname(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if host.len() > 253
        || !host.contains('.')
        || [".localhost", ".local", ".lan", ".internal"]
            .iter()
            .any(|suffix| host.ends_with(suffix))
    {
        return false;
    }
    let numeric = host.split('.').all(|label| {
        label.bytes().all(|byte| byte.is_ascii_digit())
            || label
                .strip_prefix("0x")
                .is_some_and(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
    });
    !numeric
        && host.split('.').all(|label| {
            label.len() <= 63
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn public_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    !ip.is_private()
        && !ip.is_loopback()
        && !ip.is_link_local()
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && !ip.is_broadcast()
        && a != 0
        && a < 240
        && !(a == 100 && (64..=127).contains(&b))
        && !(a == 198 && (18..=19).contains(&b))
}
pub(super) fn valid(maker: Option<&str>, seats: &[(u8, Option<u64>)]) -> bool {
    if maker.is_some_and(|maker| !shareable_matchmaker(maker)) || seats.len() > MAX_PROFILE_SEATS {
        return false;
    }
    let mut seen = [false; MAX_PROFILE_SEATS];
    for &(slot, account) in seats {
        let Some(used) = seen.get_mut(usize::from(slot)) else {
            return false;
        };
        if *used || account == Some(0) || (maker.is_none() && account.is_some()) {
            return false;
        }
        *used = true;
    }
    true
}

pub(super) fn read_matchmaker<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    struct Optional;
    impl<'de> Visitor<'de> for Optional {
        type Value = Option<String>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an optional bounded public URI")
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            struct Uri;
            impl Visitor<'_> for Uri {
                type Value = String;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("a bounded public URI")
                }
                fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
                    if !shareable_matchmaker(value) {
                        return Err(E::custom("invalid public matchmaker URI"));
                    }
                    Ok(value.to_owned())
                }
            }
            deserializer.deserialize_str(Uri).map(Some)
        }
    }
    deserializer.deserialize_option(Optional)
}
pub(super) fn read_seats<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<(u8, Option<u64>)>, D::Error> {
    struct Seats;
    impl<'de> Visitor<'de> for Seats {
        type Value = Vec<(u8, Option<u64>)>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("at most ten unique public seat accounts")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            if seq.size_hint().is_some_and(|len| len > MAX_PROFILE_SEATS) {
                return Err(de::Error::custom("too many profile seats"));
            }
            let mut seats = Vec::with_capacity(MAX_PROFILE_SEATS);
            while let Some(seat) = seq.next_element::<(u8, Option<u64>)>()? {
                if seats.len() == MAX_PROFILE_SEATS
                    || usize::from(seat.0) >= MAX_PROFILE_SEATS
                    || seat.1 == Some(0)
                    || seats
                        .iter()
                        .any(|previous: &(u8, Option<u64>)| previous.0 == seat.0)
                {
                    return Err(de::Error::custom("invalid profile seat"));
                }
                seats.push(seat);
            }
            Ok(seats)
        }
    }
    deserializer.deserialize_seq(Seats)
}

/// Enriches validated metadata without changing any of its match facts.
pub fn with_profiles(bytes: &[u8], profiles: &Profiles) -> Option<Vec<u8>> {
    let meta = super::decode(bytes).ok()?;
    if meta.damaged {
        return None;
    }
    let record = profiles.record();
    let frame = bota_proto::encode_frame_to_vec(&record).ok()?;
    let mut output = Vec::with_capacity(bytes.len().saturating_add(frame.len()));
    let mut rest = bytes;
    let mut contexts = 0;
    while !rest.is_empty() {
        let (head, tail) = rest.split_at(bota_proto::LEN_PREFIX);
        let len = u32::from_le_bytes(head.try_into().ok()?) as usize;
        let (payload, next) = tail.split_at(len);
        if let Ok(MetaRecord::Profiles { matchmaker, seats }) = bota_proto::decode_payload(payload)
        {
            contexts += 1;
            if contexts > 1 || !valid(matchmaker.as_deref(), &seats) {
                return None;
            }
        } else {
            output.extend_from_slice(head);
            output.extend_from_slice(payload);
        }
        rest = next;
    }
    if output.len().saturating_add(frame.len())
        > crate::limits::MAX_META_BYTES + crate::limits::END_RESERVE_BYTES
    {
        return None;
    }
    output.extend_from_slice(&frame);
    Some(output)
}
