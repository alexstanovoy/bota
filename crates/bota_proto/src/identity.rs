//! Public seat identity, independent of game state and credentials.
use crate::{MAX_VERSION_LEN, SlotId};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SeatIdentity {
    pub slot: SlotId,
    #[serde(deserialize_with = "crate::deserialize_seat_name")]
    pub name: String,
    pub bot: bool,
    #[serde(deserialize_with = "version")]
    pub version: Option<String>,
}

pub fn valid_version(name: &str) -> bool {
    (1..=MAX_VERSION_LEN).contains(&name.len())
        && name == name.trim()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
}

/// Public facts reject a hostile length before copying any string bytes.
pub fn version<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    use serde::de::{Error, Visitor};
    struct Optional;
    struct Name;
    impl<'de> Visitor<'de> for Name {
        type Value = String;
        fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "a valid version of at most {MAX_VERSION_LEN} bytes")
        }
        fn visit_str<E: Error>(self, value: &str) -> Result<String, E> {
            if value.len() > MAX_VERSION_LEN {
                return Err(E::invalid_length(value.len(), &self));
            }
            if !valid_version(value) {
                return Err(E::custom("invalid version"));
            }
            Ok(value.to_owned())
        }
    }
    impl<'de> Visitor<'de> for Optional {
        type Value = Option<String>;
        fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("an optional version")
        }
        fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_str(Name).map(Some)
        }
    }
    deserializer.deserialize_option(Optional)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodecError, LEN_PREFIX, decode_payload, encode_frame_to_vec};

    #[test]
    fn seat_identity_versions_round_trip_only_within_the_validated_bound() {
        for byte in 0..=255u8 {
            let version = format!("a{}b", char::from(byte));
            let identity = SeatIdentity {
                slot: SlotId(0),
                name: "engine".into(),
                bot: true,
                version: Some(version.clone()),
            };
            let frame = encode_frame_to_vec(&identity).expect("serialize");
            let decoded = decode_payload::<SeatIdentity>(&frame[LEN_PREFIX..]);
            if valid_version(&version) {
                assert_eq!(decoded, Ok(identity));
            } else {
                assert_eq!(decoded, Err(CodecError::Malformed));
            }
        }
        for version in [
            String::new(),
            "x".repeat(MAX_VERSION_LEN + 1),
            " v1".into(),
            "v1 ".into(),
        ] {
            let identity = SeatIdentity {
                slot: SlotId(0),
                name: "engine".into(),
                bot: true,
                version: Some(version),
            };
            let frame = encode_frame_to_vec(&identity).expect("serialize invalid input");
            assert_eq!(
                decode_payload::<SeatIdentity>(&frame[LEN_PREFIX..]),
                Err(CodecError::Malformed)
            );
        }
    }
}
