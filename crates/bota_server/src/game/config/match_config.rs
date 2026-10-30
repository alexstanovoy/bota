//! Everything a match is created from.

use bota_proto::{EventKind, MapId, MatchInfo, ObjectiveSet, Pick, RuneSet, Team, TickMode};

use crate::game::{
    MAX_SPAWN_MODIFIERS, MatchRng, SpawnModifier, SpawnModifierError, check_spawn_modifier,
};

/// Why a match description was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchConfigError {
    /// More spawn modifiers than a match may carry.
    TooManySpawnModifiers { count: usize },
    /// One spawn modifier is not one a match may carry.
    SpawnModifier {
        at: usize,
        error: SpawnModifierError,
    },
    /// Objectives the map holds no data for or the simulation does not play.
    Objectives { refused: ObjectiveSet, map: MapId },
    /// Runes the map has no spots for or the simulation does not play.
    Runes { refused: RuneSet, map: MapId },
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
            MatchConfigError::Objectives { refused, map } => write!(
                f,
                "map {} plays no such objectives: {refused} (it plays: {})",
                map.0,
                bota_core::map_of(*map).playable_objectives()
            ),
            MatchConfigError::Runes { refused, map } => write!(
                f,
                "map {} spawns no such runes: {refused} (it spawns: {})",
                map.0,
                bota_core::map_of(*map).playable_runes()
            ),
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
    /// Identifies this match in logs and replay files. A label only: no
    /// randomness is derived from it.
    pub match_id: u64,
    /// Server secret the match randomness is derived from.
    pub master_key: [u8; 32],
    /// Which of the key's streams the match randomness reads. Secret like the key.
    pub stream: u64,
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
    /// Whether structures keep at least 1 hp: they take damage and draw aggro
    /// as usual but never fall.
    pub immortal_structures: bool,
    /// Whether heroes keep at least 1 hp: they take damage as usual but never
    /// die, so no match ends by hero deaths.
    pub immortal_heroes: bool,
    /// Starting gold a seat loses for picking after the selection time.
    pub starting_gold_lost: Vec<(bota_proto::SlotId, i32)>,
    /// Which of the map's objectives stand: the map's
    /// [`default_objectives`](bota_core::MapDef::default_objectives) unless
    /// the host says otherwise, and never one the map does not
    /// [play](bota_core::MapDef::playable_objectives).
    pub objectives: ObjectiveSet,
    /// Which kinds of rune spawn: the map's
    /// [`default_runes`](bota_core::MapDef::default_runes) unless the host
    /// says otherwise, and never one the map does not
    /// [play](bota_core::MapDef::playable_runes).
    pub runes: RuneSet,
}

impl MatchConfig {
    /// The hidden randomness of this match.
    pub fn rng(&self) -> MatchRng {
        MatchRng::new(&self.master_key, self.stream)
    }

    /// The starting gold a seat lost to a late pick.
    pub fn gold_lost_by(&self, slot: bota_proto::SlotId) -> i32 {
        self.starting_gold_lost
            .iter()
            .filter(|(lost, _)| *lost == slot)
            .map(|(_, gold)| gold)
            .sum()
    }

    /// Whether the description may build a match. The spawn modifiers must
    /// fit the bound and every one of them must be valid, and the map must
    /// play every objective and rune asked for.
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
        check_match_rules(self.map, self.objectives, self.runes)?;
        Ok(())
    }

    /// The projection onto the wire. Carries no secret.
    pub fn info(&self) -> MatchInfo {
        let map = bota_core::map_of(self.map);
        let terrain = crate::game::terrain_of(map);
        MatchInfo {
            match_id: self.match_id,
            map: self.map,
            tick_rate: self.tick_rate,
            pregame_ticks: bota_core::rules::PREGAME_TICKS,
            trees: crate::game::tree_positions(map, terrain),
            terrain_cells: crate::game::TERRAIN_CELLS as u32,
            terrain_rle: crate::game::Ground::wire_rle(terrain),
            opaque_cells: crate::game::fow_wall_cells(terrain),
            mode: self.mode,
            picks: self.picks.clone(),
            shop: bota_core::shop_entries(),
            fountains: map.fountains,
            shop_range: bota_core::rules::SHOP_RANGE,
            cheats: self.cheats,
            objectives: self.objectives,
            runes: self.runes,
        }
    }
}

/// Whether a map plays the objectives and runes asked of it: every one it
/// holds the data for and the simulation plays, and no other.
pub fn check_match_rules(
    map: MapId,
    objectives: ObjectiveSet,
    runes: RuneSet,
) -> Result<(), MatchConfigError> {
    let def = bota_core::map_of(map);
    let refused = objectives.without(def.playable_objectives());
    if !refused.is_empty() {
        return Err(MatchConfigError::Objectives { refused, map });
    }
    let refused = runes.without(def.playable_runes());
    if !refused.is_empty() {
        return Err(MatchConfigError::Runes { refused, map });
    }
    Ok(())
}

/// One accepted order of a seat, as the world takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// Which seat gave it.
    pub slot: bota_proto::SlotId,
    /// Which unit it is for, in the simulation's handles. Absent means the
    /// seat's own hero.
    pub unit: Option<bota_proto::EntityId>,
    /// The order itself, in the simulation's handles.
    pub order: bota_proto::Order,
    /// Whether it waits its turn in the unit's queue.
    pub queue: bool,
    /// The number its seat gave it, to answer a refusal with.
    pub seq: u32,
}

impl Command {
    /// The order as the wire tells it.
    pub const fn told(&self) -> bota_proto::SlotOrder {
        bota_proto::SlotOrder {
            slot: self.slot,
            unit: self.unit,
            order: self.order,
            queue: self.queue,
        }
    }

    /// Whether it goes into its unit's queue rather than replace it: given
    /// queued, and of the orders that queue.
    pub const fn queued(&self) -> bool {
        self.queue && bota_core::order_queues(self.order)
    }
}

/// Who may learn that an event happened. Limits the player streams;
/// spectators see every event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventVisibility {
    /// Both teams.
    Everyone,
    /// One team only.
    OneTeam(Team),
    /// Neither team: something of the jungle nobody saw.
    Nobody,
}

impl EventVisibility {
    /// The audience of the two teams told: Radiant when `radiant`, Dire
    /// when `dire`.
    pub fn of(radiant: bool, dire: bool) -> EventVisibility {
        match (radiant, dire) {
            (true, true) => EventVisibility::Everyone,
            (true, false) => EventVisibility::OneTeam(Team::Radiant),
            (false, true) => EventVisibility::OneTeam(Team::Dire),
            (false, false) => EventVisibility::Nobody,
        }
    }

    /// Whether a team is among those told.
    pub fn tells(self, team: Team) -> bool {
        match self {
            EventVisibility::Everyone => true,
            EventVisibility::OneTeam(told) => told == team,
            EventVisibility::Nobody => false,
        }
    }
}

/// One thing that happened during a tick, with its audience.
#[derive(Clone, Debug)]
pub struct Event {
    /// What happened.
    pub kind: EventKind,
    /// Who may know.
    pub visible_to: EventVisibility,
    /// The side that must see zero for the gold a `Died` reports: the one
    /// that did not earn a non-hero kill. A hero kill's gold is public, as in
    /// Dota's kill feed, so it is `None` there and for every other event.
    pub gold_hidden_from: Option<Team>,
    /// The teams told who the event names as its dealer: the `source` of a
    /// `Damaged`, `Missed` or `Healed`, the `killer` of a `Died`. The others
    /// are told `None`: a team learns who it does not see only from what
    /// Dota announces, a hero's or a tower's killer. `Everyone` for an event
    /// that names no dealer.
    pub source_told_to: EventVisibility,
}
