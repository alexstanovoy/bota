//! Everything a match is created from.

use bota_proto::{EntityId, EventKind, MapId, MatchInfo, Order, Pick, SlotId, Team, TickMode};

use crate::game::MatchRng;

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
}

impl MatchConfig {
    /// The hidden randomness of this match.
    pub fn rng(&self) -> MatchRng {
        MatchRng::new(&self.master_key, self.match_id)
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
