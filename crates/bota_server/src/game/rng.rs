//! Hidden randomness. Nothing here ever reaches a participant.

use bota_core::Odds;
use bota_proto::{EntityId, SlotId, Team};
use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{Rng as _, SeedableRng};

use crate::engine::{Fnv, StateHash, state_hash};

/// What a stream feeds. Each purpose draws from a stream of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum Purpose {
    /// Evasion and uphill-miss ordering.
    Evasion = 2,
    /// Which roster a neutral camp puts out.
    NeutralSpawn = 4,
    /// Which melee creep of a wave carries the flag.
    Wave = 5,
    /// Pierce ordering.
    Pierce = 6,
    /// What each kill paying a gold range pays.
    Bounty = 7,
    /// What each swing with a damage range deals.
    DamageRoll = 8,
    /// The order the blows landing in one tick are felt in.
    HitOrder = 9,
    /// Which way a tick takes its entities: up or down the slots.
    TickOrder = 10,
    /// Which swings an accuracy aura lets through evasion.
    Accuracy = 11,
    /// Critical strike ordering.
    Crit = 12,
    /// Whether a neutral answers the death of one of its camp with a cast.
    NeutralCast = 13,
    /// Which of the heroes gathering a Shrine of Wisdom it rewards. Drawn
    /// from the Shrine's own stream of each activation only.
    Shrine = 14,
    /// Which power rune and which river spot a river spawn puts out. Drawn
    /// from the stream of each spawn alone, keyed by its count.
    Rune = 15,
}

/// The purposes with a stream of the match as a whole, whose draws the
/// world's fingerprint counts: every one up to [`Purpose::NeutralCast`].
/// One after it draws from the streams of units alone
/// ([`MatchRng::for_unit`]), which are drawn afresh and keep no count.
const PURPOSE_COUNT: usize = Purpose::NeutralCast as usize + 1;

/// The root of all hidden randomness in one match, reproducible from the
/// pair of a server key and a stream number.
#[derive(Clone, Debug)]
pub struct MatchRng {
    seed: [u8; 32],
    global: [Stream; PURPOSE_COUNT],
}

impl MatchRng {
    /// Derives the randomness of one match.
    pub fn new(master_key: &[u8; 32], stream: u64) -> MatchRng {
        let mut root = ChaCha8Rng::from_seed(*master_key);
        root.set_stream(stream);
        let mut seed = [0u8; 32];
        root.fill_bytes(&mut seed);
        let global = std::array::from_fn(|purpose| {
            open_stream(seed, GLOBAL_BIT | ((purpose as u64) << PURPOSE_SHIFT))
        });
        MatchRng { seed, global }
    }

    /// A stream that belongs to the match as a whole.
    pub fn global(&mut self, purpose: Purpose) -> &mut Stream {
        assert!(
            (purpose as usize) < PURPOSE_COUNT,
            "{purpose:?} draws from the streams of units alone"
        );
        &mut self.global[purpose as usize]
    }

    /// A fresh stream that belongs to one unit and one of its sources of
    /// chance. `source` separates several sources on the same unit. Keyed by
    /// the slot index, not the generation: every occupant of a slot gets the
    /// same stream.
    pub fn for_unit(&self, purpose: Purpose, unit: EntityId, source: u8) -> Stream {
        open_stream(
            self.seed,
            ((purpose as u64) << PURPOSE_SHIFT) | ((unit.idx as u64) << UNIT_SHIFT) | source as u64,
        )
    }

    /// A fresh stream that holds back one seat's orders in flight. Apart
    /// from every stream of the simulation itself.
    pub fn for_order_delay(&self, slot: SlotId) -> Stream {
        open_stream(self.seed, ORDER_DELAY_BIT | u64::from(slot.0))
    }

    /// A fresh stream that orders the names one side is given for the
    /// entities it is told of first on the same tick. Apart from every
    /// stream of the simulation itself.
    pub fn for_handles(&self, team: Team) -> Stream {
        open_stream(self.seed, HANDLES_BIT | team as u64)
    }

    /// The root seed every stream is derived from.
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
const ORDER_DELAY_BIT: u64 = 1 << 62;
const HANDLES_BIT: u64 = 1 << 61;
const _: () = assert!(((PURPOSE_COUNT as u64) << PURPOSE_SHIFT) <= HANDLES_BIT);
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
        // Reject the tail that would make low values more likely.
        let zone = ((1u64 << 32) / n as u64) * n as u64;
        loop {
            let v = self.next_u32() as u64;
            if v < zone {
                return (v % n as u64) as u32;
            }
        }
    }
}

/// One source of chance rolling with Dota's pseudo-random distribution: the
/// `n`-th attempt since its last hit hits with `n` times the step of its
/// chance ([`Odds::prd_step`]), so a run of misses grows ever likelier to end
/// and over many attempts it hits at its chance. Its draws come from a hidden
/// stream of its own.
///
/// A chance that changes keeps the run of misses so far: the next attempt
/// rolls with the new chance's step.
#[derive(Clone, Debug)]
pub struct PseudoRandom {
    stream: Stream,
    /// Attempts missed since the last hit.
    misses: u32,
    /// The chance last rolled, and its step.
    step: (Odds, u64),
}

impl PseudoRandom {
    /// A source drawing from `stream`, as if it had just hit.
    pub fn new(stream: Stream) -> PseudoRandom {
        PseudoRandom {
            stream,
            misses: 0,
            step: (Odds::NEVER, 0),
        }
    }

    /// Whether this attempt hits at `odds`. A chance that never hits rolls
    /// nothing and leaves the run as it is.
    pub fn roll(&mut self, odds: Odds) -> bool {
        if odds == Odds::NEVER {
            return false;
        }
        if self.step.0 != odds {
            self.step = (odds, odds.prd_step());
        }
        const DRAW_SPACE: u64 = 1 << 32;
        let threshold = self.step.1.saturating_mul(u64::from(self.misses) + 1);
        let hits = threshold >= DRAW_SPACE || u64::from(self.stream.next_u32()) < threshold;
        self.misses = if hits { 0 } else { self.misses + 1 };
        hits
    }

    /// Draw count and run of misses determining the next outcome.
    pub fn state(&self) -> (u64, u32) {
        (self.stream.draws, self.misses)
    }
}

/// One entity's sources of one kind of chance, by the place each is given
/// at: an item's slot, an ability's.
#[derive(Clone, Debug, Default)]
pub struct Rolls(Vec<Option<PseudoRandom>>);

impl Rolls {
    /// The source at a place, opened with `open` the first time it rolls.
    pub fn at(&mut self, place: usize, open: impl FnOnce() -> PseudoRandom) -> &mut PseudoRandom {
        if self.0.len() <= place {
            self.0.resize_with(place + 1, || None);
        }
        self.0[place].get_or_insert_with(open)
    }

    /// Every source opened, by place.
    pub fn sources(&self) -> &[Option<PseudoRandom>] {
        &self.0
    }
}

/// The key is where every stream comes from, and a stream's place is its draw count.
impl StateHash for MatchRng {
    fn state_hash(&self, fnv: &mut Fnv) {
        let MatchRng { seed, global } = self;
        seed.state_hash(fnv);
        global.state_hash(fnv);
    }
}

/// The generator of a stream is fixed by the seed, the stream's number and the draws made from it.
impl StateHash for Stream {
    fn state_hash(&self, fnv: &mut Fnv) {
        let Stream { inner: _, draws } = self;
        draws.state_hash(fnv);
    }
}

/// The step is the chance last rolled worked out, and follows from it.
impl StateHash for PseudoRandom {
    fn state_hash(&self, fnv: &mut Fnv) {
        let PseudoRandom {
            stream,
            misses,
            step: _,
        } = self;
        stream.state_hash(fnv);
        misses.state_hash(fnv);
    }
}

state_hash!(Rolls(0));

#[cfg(test)]
mod specimens {
    use super::*;
    use crate::engine::Specimen;

    impl Specimen for Stream {
        fn low() -> Self {
            open_stream([0; 32], 1)
        }

        fn high() -> Self {
            Stream {
                draws: 5,
                ..open_stream([0; 32], 1)
            }
        }
    }

    impl Specimen for PseudoRandom {
        fn low() -> Self {
            PseudoRandom::new(Stream::low())
        }

        fn high() -> Self {
            PseudoRandom {
                stream: Stream::high(),
                misses: 3,
                step: (Odds::NEVER, 0),
            }
        }
    }

    impl Specimen for MatchRng {
        fn low() -> Self {
            MatchRng::new(&[0; 32], 0)
        }

        fn high() -> Self {
            let mut rng = MatchRng::new(&[1; 32], 1);
            rng.global(Purpose::Wave).next_u32();
            rng
        }
    }
}
