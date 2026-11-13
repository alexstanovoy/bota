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
    /// Any skipped or broken frame makes the provenance ambiguous, even if statistics survive.
    pub fn profiles(&self) -> Option<Profiles> {
        if self.skipped != 0 {
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
pub(super) fn valid(maker: Option<&str>, seats: &[(u8, Option<u64>)]) -> bool {
    if maker.is_some_and(|maker| !public_matchmaker(maker)) || seats.len() > MAX_PROFILE_SEATS {
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
                    if !public_matchmaker(value) {
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
    if meta.skipped != 0 {
        return None;
    }
    let mut writer = super::Writer::default();
    if !writer.push(&MetaRecord::Header(meta.header), true) {
        return None;
    }
    let mut contexts = 0;
    for record in &meta.records {
        if let MetaRecord::Profiles { matchmaker, seats } = record {
            contexts += 1;
            if contexts > 1 || !valid(matchmaker.as_deref(), seats) {
                return None;
            }
        } else if !writer.push(record, true) {
            return None;
        }
    }
    if !writer.push(&profiles.record(), true) {
        return None;
    }
    Some(writer.into_bytes())
}
