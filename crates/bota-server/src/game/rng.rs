//! Hidden randomness.
//!
//! Nothing here ever reaches a participant. A client that could read a stream
//! position could name the tick of the next critical strike.

use bota_proto::EntityId;
use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{Rng as _, SeedableRng};

/// What a stream feeds.
///
/// Streams are kept apart by purpose so that adding a draw in one place does not
/// shift the sequence anywhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum Purpose {
    /// Critical strike ordering.
    Crit = 0,
    /// Damage block ordering.
    Block = 1,
    /// Evasion ordering.
    Evasion = 2,
    /// Which rune spawns.
    Rune = 3,
    /// Scatter of neutral camp spawns.
    NeutralSpawn = 4,
    /// Which melee creep of a wave carries the flag.
    Wave = 5,
}

const PURPOSE_COUNT: usize = Purpose::Wave as usize + 1;

/// The root of all hidden randomness in one match.
///
/// Reproducible from the pair of a server key and a match id, which is what
/// makes a reported match replayable without storing anything secret.
#[derive(Clone, Debug)]
pub struct MatchRng {
    seed: [u8; 32],
    global: [Stream; PURPOSE_COUNT],
}

impl MatchRng {
    /// Derives the randomness of one match.
    pub fn new(master_key: &[u8; 32], match_id: u64) -> MatchRng {
        let mut root = ChaCha8Rng::from_seed(*master_key);
        root.set_stream(match_id);
        let mut seed = [0u8; 32];
        root.fill_bytes(&mut seed);
        let global = std::array::from_fn(|purpose| {
            open_stream(seed, GLOBAL_BIT | ((purpose as u64) << PURPOSE_SHIFT))
        });
        MatchRng { seed, global }
    }

    /// A stream that belongs to the match as a whole.
    pub fn global(&mut self, purpose: Purpose) -> &mut Stream {
        &mut self.global[purpose as usize]
    }

    /// A stream that belongs to one unit and one of its sources of chance.
    ///
    /// `source` separates several sources on the same unit, such as a crit
    /// passive and a bash. Streams are keyed by slot index rather than by whole
    /// [`EntityId`], so a unit that takes over a freed slot continues the
    /// sequence rather than restarting it.
    pub fn for_unit(&self, purpose: Purpose, unit: EntityId, source: u8) -> Stream {
        self.open(
            ((purpose as u64) << PURPOSE_SHIFT) | ((unit.idx as u64) << UNIT_SHIFT) | source as u64,
        )
    }

    fn open(&self, stream_id: u64) -> Stream {
        open_stream(self.seed, stream_id)
    }

    /// Root seed used to derive every stream.
    pub fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    /// Draw counts of match-global streams in purpose order.
    pub fn global_draws(&self) -> impl Iterator<Item = u64> + '_ {
        self.global.iter().map(|stream| stream.draws)
    }
}

fn open_stream(seed: [u8; 32], stream_id: u64) -> Stream {
    let mut inner = ChaCha8Rng::from_seed(seed);
    inner.set_stream(stream_id);
    Stream { inner, draws: 0 }
}

const GLOBAL_BIT: u64 = 1 << 63;
const PURPOSE_SHIFT: u32 = 48;
const UNIT_SHIFT: u32 = 8;

/// One independent sequence of hidden draws.
#[derive(Clone, Debug)]
pub struct Stream {
    inner: ChaCha8Rng,
    draws: u64,
}

impl Stream {
    /// The next value.
    pub fn next_u32(&mut self) -> u32 {
        self.draws = self
            .draws
            .checked_add(1)
            .expect("random draw counter overflow");
        self.inner.next_u32()
    }

    /// A value in `0..n`, drawn without bias.
    ///
    /// Panics when `n` is zero.
    pub fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0, "below(0) has no answer");
        // Reject the tail that would make low values more likely. Rejection
        // consumes from the stream, so it stays reproducible.
        let zone = ((1u64 << 32) / n as u64) * n as u64;
        loop {
            let v = self.next_u32() as u64;
            if v < zone {
                return (v % n as u64) as u32;
            }
        }
    }
}

/// Dota's pseudo-random distribution for one 25% event source.
#[derive(Clone, Debug)]
pub struct PseudoRandom25 {
    stream: Stream,
    failures: u8,
}

impl PseudoRandom25 {
    /// Starts a sequence with its lowest per-attempt chance.
    pub fn new(stream: Stream) -> Self {
        Self {
            stream,
            failures: 0,
        }
    }

    /// Whether the event occurs on this eligible attempt.
    pub fn roll(&mut self) -> bool {
        const BASE_THRESHOLD: u64 = 363_973_103;
        const DRAW_SPACE: u64 = 1u64 << 32;

        let attempt = u64::from(self.failures) + 1;
        let threshold = BASE_THRESHOLD * attempt;
        let occurs = threshold >= DRAW_SPACE || u64::from(self.stream.next_u32()) < threshold;
        if occurs {
            self.failures = 0;
        } else {
            self.failures = self.failures.saturating_add(1);
        }
        debug_assert!(self.failures < 12);
        occurs
    }

    /// Draw count and failure streak determining the next outcome.
    pub fn state(&self) -> (u64, u8) {
        (self.stream.draws, self.failures)
    }
}

/// An exact rate, written as a fraction of attempts.
///
/// Balance constants are fractions rather than percentages because the rate is
/// honoured exactly: `Ratio::new(3, 10)` hits three times in every ten
/// attempts, not three times on average.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ratio {
    num: u8,
    den: u8,
}

impl Ratio {
    /// Largest denominator a [`Chance`] can carry, set by the width of its mask.
    pub const MAX_DEN: u8 = 64;

    /// Never hits.
    pub const NEVER: Ratio = Ratio { num: 0, den: 1 };

    /// Always hits.
    pub const ALWAYS: Ratio = Ratio { num: 1, den: 1 };

    /// `num` hits out of every `den` attempts.
    ///
    /// Panics unless `0 < den <= MAX_DEN` and `num <= den`.
    pub const fn new(num: u8, den: u8) -> Ratio {
        assert!(den > 0, "a ratio needs a denominator");
        assert!(den <= Ratio::MAX_DEN, "denominator above MAX_DEN");
        assert!(num <= den, "more hits than attempts");
        Ratio { num, den }
    }

    /// Hits per block.
    pub const fn num(self) -> u8 {
        self.num
    }

    /// Attempts per block.
    pub const fn den(self) -> u8 {
        self.den
    }
}

/// A source of chance that honours its [`Ratio`] exactly while hiding its order.
///
/// Every block of `den` attempts contains exactly `num` hits. Which attempts
/// those are comes from a hidden stream, so watching past outcomes says nothing
/// about the next block. The first block starts at an offset drawn from the same
/// stream and is therefore short, which is what keeps block boundaries from
/// being countable.
///
/// A [`Ratio`] passed to [`roll`](Chance::roll) takes effect at the next block
/// boundary. The block in progress finishes under the ratio it started with.
#[derive(Clone, Debug)]
pub struct Chance {
    stream: Stream,
    /// Which attempts of the current block hit, one bit each.
    mask: u64,
    /// Position within the current block.
    idx: u8,
    /// The ratio the current block was built with.
    current: Ratio,
}

impl Chance {
    /// A source drawing from `stream`, starting mid-block.
    pub fn new(mut stream: Stream, ratio: Ratio) -> Chance {
        let mask = Chance::pick(&mut stream, ratio);
        let idx = stream.below(ratio.den() as u32) as u8;
        Chance {
            stream,
            mask,
            idx,
            current: ratio,
        }
    }

    /// Whether this attempt hits.
    pub fn roll(&mut self, ratio: Ratio) -> bool {
        if self.idx >= self.current.den() {
            self.current = ratio;
            self.idx = 0;
            self.mask = Chance::pick(&mut self.stream, ratio);
        }
        let hit = self.mask & (1u64 << self.idx) != 0;
        self.idx += 1;
        hit
    }

    /// The ratio the block in progress was built with.
    pub fn current(&self) -> Ratio {
        self.current
    }

    /// How many attempts of the current block have been spent.
    ///
    /// Zero means the next [`roll`](Chance::roll) opens a fresh block.
    pub fn block_position(&self) -> u8 {
        if self.idx >= self.current.den() {
            0
        } else {
            self.idx
        }
    }

    /// Chooses which `num` of the `den` attempts in a block hit.
    fn pick(stream: &mut Stream, ratio: Ratio) -> u64 {
        let den = ratio.den() as usize;
        let num = ratio.num() as usize;
        if num == 0 {
            return 0;
        }
        if num == den {
            return if den == 64 {
                u64::MAX
            } else {
                (1u64 << den) - 1
            };
        }

        let mut positions = [0u8; Ratio::MAX_DEN as usize];
        for (i, p) in positions.iter_mut().enumerate().take(den) {
            *p = i as u8;
        }

        // Partial Fisher-Yates: only the first `num` draws are needed.
        let mut mask = 0u64;
        for i in 0..num {
            let j = i + stream.below((den - i) as u32) as usize;
            positions.swap(i, j);
            mask |= 1u64 << positions[i];
        }
        mask
    }
}
