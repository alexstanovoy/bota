//! Everything a match is created from.

use bota_proto::{EntityId, EventKind, MapId, MatchInfo, Order, Pick, SlotId, Team, TickMode};

use crate::game::{
    MAX_SPAWN_MODIFIERS, MatchRng, SpawnModifier, SpawnModifierError, check_spawn_modifier,
};

/// Why a match description was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchConfigError {
    /// More spawn modifiers than a match may carry.
    TooManySpawnModifiers {
        /// How many were offered.
        count: usize,
    },
    /// One spawn modifier is not one a match may carry.
    SpawnModifier {
        /// Where it sits in the list.
        at: usize,
        /// Why it was refused.
        error: SpawnModifierError,
    },
}

impl core::fmt::Display for MatchConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MatchConfigError::TooManySpawnModifiers { count } => write!(
                f,
                "{count} spawn modifiers exceed the {MAX_SPAWN_MODIFIERS} a match may carry"
            ),
            MatchConfigError::SpawnModifier { at, error } => {
                write!(f, "spawn modifier {at} is refused: {error}")
            }
        }
    }
}

impl std::error::Error for MatchConfigError {}

/// Full server-side description of one match.
///
/// [`MatchConfig::info`] is the part a participant may see; the rest never
/// leaves the server.
#[derive(Clone, Debug)]
pub struct MatchConfig {
    /// Identifies this match in logs and replay files.
    pub match_id: u64,
    /// Server secret the match randomness is derived from.
    pub master_key: [u8; 32],
    /// Every seat and its hero, sorted by slot.
    pub picks: Vec<Pick>,
    /// Which map is being played.
    pub map: MapId,
    /// Simulation ticks per second of wall-clock time.
    pub tick_rate: u16,
    /// How the server advances ticks.
    pub mode: TickMode,
    /// In lockstep, how long to wait for an acknowledgement before advancing
    /// with an empty order, in ticks of wall-clock time at `tick_rate`.
    pub ack_timeout_ticks: u32,
    /// Whether cheat orders are honoured.
    pub cheats: bool,
    /// Trusted modifiers put on units as they are stood up, and on those
    /// already standing when the match begins. `MAX_SPAWN_MODIFIERS` at most.
    pub spawn_modifiers: Vec<SpawnModifier>,
}

impl MatchConfig {
    /// The hidden randomness of this match.
    pub fn rng(&self) -> MatchRng {
        MatchRng::new(&self.master_key, self.match_id)
    }

    /// Whether the description may build a match. The spawn modifiers must
    /// fit the bound and every one of them must be valid.
    pub fn validate(&self) -> Result<(), MatchConfigError> {
        if self.spawn_modifiers.len() > MAX_SPAWN_MODIFIERS {
            return Err(MatchConfigError::TooManySpawnModifiers {
                count: self.spawn_modifiers.len(),
            });
        }
        for (at, rule) in self.spawn_modifiers.iter().enumerate() {
            check_spawn_modifier(rule)
                .map_err(|error| MatchConfigError::SpawnModifier { at, error })?;
        }
        Ok(())
    }

    /// The projection onto the wire. Carries no secret.
    pub fn info(&self) -> MatchInfo {
        MatchInfo {
            match_id: self.match_id,
            map: self.map,
            tick_rate: self.tick_rate,
            pregame_ticks: crate::game::rules::PREGAME_TICKS,
            trees: crate::game::tree_positions(crate::game::map_of(self.map)),
            terrain_cells: crate::game::TERRAIN_CELLS as u32,
            terrain_rle: crate::game::Ground::wire_rle(crate::game::map_of(self.map)),
            opaque_cells: crate::game::sight_block_cells(crate::game::map_of(self.map)),
            mode: self.mode,
            picks: self.picks.clone(),
            shop: crate::game::shop_entries(),
        }
    }
}

/// One accepted order, translated to a seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// Which seat issued it.
    pub slot: SlotId,
    /// Which unit it is for. Absent means that seat's own hero.
    pub unit: Option<EntityId>,
    /// What it asks for.
    pub order: Order,
}

/// Who may learn that an event happened.
///
/// Spectators and the replay always see everything; this limits the player
/// streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventVisibility {
    /// Both teams.
    Everyone,
    /// One team only.
    OneTeam(Team),
}

/// One thing that happened during a tick, with its audience.
#[derive(Clone, Debug)]
pub struct Event {
    /// What happened.
    pub kind: EventKind,
    /// Who may know.
    pub visible_to: EventVisibility,
}
