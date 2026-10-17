//! What a match's randomness is made of, and the public label it goes by.

use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{Rng as _, SeedableRng};

/// The key's stream the match randomness reads.
const MATCH_STREAM: u64 = 0;
/// The key's stream the label of a match is drawn from.
const LABEL_STREAM: u64 = 1;
/// The key's stream heroes are drawn from for seats nobody picked for.
const HERO_STREAM: u64 = 2;
/// The stream that turns a seed into a key.
const SEED_STREAM: u64 = 3;

/// The secret key of a match's randomness and the public label it goes by.
///
/// Every published number comes out of the key one way, through ChaCha8: the
/// label is a block of one stream, the randomness reads another. Neither the
/// label nor anything the match shows gives the key away, and with it the
/// seed it came from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MatchSecrets {
    /// The key all of the match's randomness comes from.
    pub master_key: [u8; 32],
    /// Which of the key's streams the randomness reads.
    pub stream: u64,
    /// The public label of the match.
    pub match_id: u64,
}

impl MatchSecrets {
    /// The secrets of a fresh key, as drawn from the operating system.
    pub fn of_key(master_key: [u8; 32]) -> MatchSecrets {
        MatchSecrets {
            master_key,
            stream: MATCH_STREAM,
            match_id: key_stream(master_key, LABEL_STREAM).next_u64(),
        }
    }

    /// The secrets a seed gives: the same seed always the same ones, so a
    /// seeded match can be played again.
    pub fn seeded(seed: u64) -> MatchSecrets {
        let mut seed_key = [0u8; 32];
        seed_key[..8].copy_from_slice(&seed.to_le_bytes());
        let mut master_key = [0u8; 32];
        key_stream(seed_key, SEED_STREAM).fill_bytes(&mut master_key);
        MatchSecrets::of_key(master_key)
    }

    /// The stream heroes are drawn from for seats nobody picked for.
    pub fn hero_stream(&self) -> ChaCha8Rng {
        key_stream(self.master_key, HERO_STREAM)
    }
}

impl core::fmt::Debug for MatchSecrets {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "MatchSecrets {{ match_id: {}, .. }}", self.match_id)
    }
}

fn key_stream(key: [u8; 32], stream: u64) -> ChaCha8Rng {
    let mut rng = ChaCha8Rng::from_seed(key);
    rng.set_stream(stream);
    rng
}
