//! Collections that refuse to grow past a bound while they are read.
//!
//! A byte of postcard can stand for an element many times its size in memory
//! (a missing item is one byte and about thirty in a `Vec<Option<ItemView>>`),
//! so a frame within [`MAX_PAYLOAD_LEN`](crate::MAX_PAYLOAD_LEN) could make a
//! reader allocate far more than its size before any check of the decoded
//! message ran. The fields of the wire types name their bound from
//! [`limits`](crate::limits) with `deserialize_with`, keep the type `Vec`, and
//! are refused before the first element is read when the sequence declares
//! more, or on the element past the bound when the format cannot say.

use core::fmt;
use core::marker::PhantomData;

use serde::de::{Deserialize, Deserializer, Error, SeqAccess, Visitor};

/// Bytes a sequence reserves up front at most: what a hostile length may make
/// a reader allocate before a single element has been read.
const RESERVE_BYTES: usize = 4096;

/// A sequence of at most `MAX` elements: the type the bounded fields read
/// through.
struct Capped<T, const MAX: usize>(Vec<T>);

impl<'de, T: Deserialize<'de>, const MAX: usize> Deserialize<'de> for Capped<T, MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Bound<T, const MAX: usize>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>, const MAX: usize> Visitor<'de> for Bound<T, MAX> {
            type Value = Vec<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a sequence of at most {MAX} elements")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
                let declared = seq.size_hint();
                if let Some(len) = declared.filter(|len| *len > MAX) {
                    return Err(A::Error::invalid_length(len, &self));
                }
                let room = RESERVE_BYTES / size_of::<T>().max(1);
                let mut items = Vec::with_capacity(declared.unwrap_or(0).min(room));
                while let Some(item) = seq.next_element()? {
                    if items.len() == MAX {
                        return Err(A::Error::invalid_length(MAX + 1, &self));
                    }
                    items.push(item);
                }
                Ok(items)
            }
        }

        deserializer
            .deserialize_seq(Bound::<T, MAX>(PhantomData))
            .map(Capped)
    }
}

/// A `Vec` of at most `MAX` elements.
pub(crate) fn seq<'de, D, T, const MAX: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Capped::<T, MAX>::deserialize(deserializer).map(|capped| capped.0)
}

/// An optional `Vec` of at most `MAX` elements.
pub(crate) fn option_seq<'de, D, T, const MAX: usize>(
    deserializer: D,
) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<Capped<T, MAX>>::deserialize(deserializer).map(|capped| capped.map(|capped| capped.0))
}

/// A `Vec` of at most `OUTER` vectors of at most `INNER` elements each.
pub(crate) fn seq_of_seq<'de, D, T, const OUTER: usize, const INNER: usize>(
    deserializer: D,
) -> Result<Vec<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Capped::<Capped<T, INNER>, OUTER>::deserialize(deserializer)
        .map(|outer| outer.0.into_iter().map(|inner| inner.0).collect())
}
