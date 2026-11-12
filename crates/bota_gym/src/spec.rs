//! Everything one gym match is built from.

use bota_core::Catalogue;
use bota_proto::{Cheat, MapId, ObjectiveSet, Pick, RuneSet};
use bota_server::game::{EconomyEvent, OrderDelay, SpawnModifier};

/// Upper bound on [`MatchSpec::tick_limit`]: about 38 hours of play at 30 ticks a second.
pub const MAX_TICK_LIMIT: u32 = 1 << 22;
/// Upper bound on [`MatchSpec::setup`].
pub const MAX_SETUP_CHEATS: usize = 64;

/// A cheat order the gym gives from `seat` before the first decision, as a scenario's setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetupCheat {
    pub seat: usize,
    pub cheat: Cheat,
}

/// One match: map, seats, rules and setup. Heroes, map and limits are all parameters.
///
/// The match's identity and every random draw of the simulation come from `seed` alone: two
/// matches of one spec play out identically under the same orders. That makes a gym match's
/// randomness a public function of its seed, which is what training and evaluation want and
/// what a served match must never do (a server keeps its key secret).
#[derive(Clone, Debug)]
pub struct MatchSpec {
    pub seed: u64,
    pub map: MapId,
    /// Every seat in slot order: its slot, side and hero.
    pub picks: Vec<Pick>,
    /// How late orders take effect: none, one for every seat, or one per seat in slot order.
    pub order_delays: Vec<OrderDelay>,
    /// Objectives that stand; the map's default by [`MatchSpec::new`].
    pub objectives: ObjectiveSet,
    /// Runes that spawn; the map's default by [`MatchSpec::new`].
    pub runes: RuneSet,
    /// The items the match plays with.
    pub catalogue: Catalogue,
    /// Structures take damage but never fall.
    pub immortal_structures: bool,
    /// Heroes take damage but never die.
    pub immortal_heroes: bool,
    /// Trusted setup: modifiers put on the units their selectors name as they stand up.
    pub spawn_modifiers: Vec<SpawnModifier>,
    /// Trusted setup: gold and items handed to seats at their ticks.
    pub economy_events: Vec<EconomyEvent>,
    /// A scenario through cheat orders, applied before the first decision. Any cheat turns the
    /// match into a cheat lobby, which every seat is told in its `MatchStart`.
    pub setup: Vec<SetupCheat>,
    /// The tick the game is cut at, at the latest, if nothing ends it before;
    /// `1..=MAX_TICK_LIMIT`.
    pub tick_limit: u32,
    /// Keep bota_replay's match metadata (per-minute samples, kills, purchases, structures),
    /// returned by [`Game::finish`](crate::Game::finish). It records the whole map every tick,
    /// so it costs simulation time: for evaluation, not for training.
    pub record_meta: bool,
}

impl MatchSpec {
    /// A match on `map` with `picks`, the map's default objectives and runes, the full catalogue,
    /// no order delay, no setup and no metadata.
    pub fn new(map: MapId, picks: Vec<Pick>, seed: u64, tick_limit: u32) -> Self {
        let def = bota_core::map_of(map);
        Self {
            seed,
            map,
            picks,
            order_delays: Vec::new(),
            objectives: def.default_objectives,
            runes: def.default_runes,
            catalogue: Catalogue::FULL,
            immortal_structures: false,
            immortal_heroes: false,
            spawn_modifiers: Vec::new(),
            economy_events: Vec::new(),
            setup: Vec::new(),
            tick_limit,
            record_meta: false,
        }
    }

    /// The largest delay any seat's order can take, in ticks.
    pub(crate) fn longest_delay(&self) -> u32 {
        self.order_delays
            .iter()
            .map(|delay| delay.base.saturating_add(delay.jitter))
            .max()
            .unwrap_or(0)
    }
}
