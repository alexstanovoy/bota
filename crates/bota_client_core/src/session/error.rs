//! Why a session refused its input.

use core::convert::Infallible;
use core::fmt;
use std::error::Error;

use bota_proto::{EntityId, HeroId, Team, TickMode};

use crate::limits::{MAX_RESTORATION_REPORT, MAX_SEATS, MAX_TERRAIN_AXIS};
use crate::match_static::RawExtent;

/// A rejected session input: match terms, a snapshot, an event batch, an
/// order or the order of the stream itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// A manual restoration report was negative or outside the accepted pool bound.
    RestorationReport { channel: &'static str, amount: i32 },
    /// A bounded vector exceeded its contract.
    LimitExceeded {
        field: &'static str,
        actual: usize,
        limit: usize,
    },
    /// Match tick rate was zero.
    ZeroTickRate,
    /// Seat count was outside the supported interval.
    SeatCount(usize),
    /// Own seat was outside the supported interval.
    OwnSlotOutOfRange(u8),
    /// Match metadata did not contain the own seat.
    MissingOwnPick(u8),
    /// Own seat did not pick the hero the client plays.
    WrongOwnHero { actual: HeroId, expected: HeroId },
    /// Own seat had no playable team.
    InvalidOwnTeam(Team),
    /// Match metadata contained a repeated slot.
    DuplicatePickSlot(u8),
    /// Match metadata contained an out-of-range slot.
    PickSlotOutOfRange(u8),
    /// Terrain axis was outside the supported interval.
    TerrainAxis(u32),
    /// Terrain run lengths did not cover the terrain grid.
    TerrainCoverage { actual: u64, expected: u64 },
    /// A terrain run had zero length.
    ZeroTerrainRun(usize),
    /// An opaque cell was outside the terrain grid.
    OpaqueCellOutOfRange { x: u16, y: u16, cells: u32 },
    /// A geometry input was outside its allowed fixed-point extent.
    PositionOutOfMap {
        field: &'static str,
        index: usize,
        x_raw: i32,
        y_raw: i32,
        extent: RawExtent,
    },
    /// Snapshot viewer was absent or belonged to a different team.
    WrongViewer(Option<Team>, Team),
    /// Snapshot tick did not strictly increase.
    SnapshotTick { incoming: u32, current: u32 },
    /// A snapshot's skipped ticks were not strictly increasing between the last snapshot or
    /// events and itself.
    SkippedTicks { incoming: u32 },
    /// Snapshot omitted the own scoreboard row.
    MissingOwnPlayer(u8),
    /// Own gold was hidden in a seat-specific snapshot.
    MissingOwnGold,
    /// Own stash was hidden in a seat-specific snapshot.
    MissingOwnStash,
    /// Own live hero handle was absent from visible units.
    MissingOwnHero(EntityId),
    /// Own live hero unit had inconsistent identity data.
    InvalidOwnHero(EntityId),
    /// A visible hero claimed the own seat without matching its scoreboard body.
    AmbiguousOwnHero {
        scoreboard: Option<EntityId>,
        visible: EntityId,
    },
    /// More than one visible courier claimed the own seat.
    AmbiguousOwnCourier { first: EntityId, second: EntityId },
    /// A full entity handle occurred more than once in visible units.
    DuplicateUnit(EntityId),
    /// A full entity handle occurred more than once in visible projectiles.
    DuplicateProjectile(EntityId),
    /// A full entity handle occurred more than once in visible loot.
    DuplicateLoot(EntityId),
    /// A full seat handle occurred more than once in scoreboard rows.
    DuplicatePlayer(u8),
    /// Scoreboard rows were not sorted by slot.
    UnsortedPlayers,
    /// Talent rows were not strictly sorted by slot, or named a seat with no scoreboard row.
    UnsortedTalents,
    /// A match roster row was absent from the scoreboard.
    MissingRosterPlayer(u8),
    /// A scoreboard row did not occur in the match roster.
    UnknownRosterPlayer(u8),
    /// A scoreboard row had a team different from the match roster.
    WrongRosterTeam {
        slot: u8,
        actual: Team,
        expected: Team,
    },
    /// A scoreboard row had a hero different from the match roster.
    WrongRosterHero {
        slot: u8,
        actual: HeroId,
        expected: HeroId,
    },
    /// Visible units were not sorted by full entity handle.
    UnsortedUnits,
    /// Aggro entries were not sorted strictly by unit handle.
    UnsortedAggro,
    /// An aggro entry named a unit absent from the visible units.
    UnknownAggroUnit(EntityId),
    /// An aggro entry of this unit held a timer past its rule's bound.
    AggroOutOfRange(EntityId),
    /// A tree index did not refer to static match trees.
    InvalidFelledTree(u32),
    /// Position displacement did not fit the wire vector type.
    PositionDeltaOverflow(EntityId),
    /// Event batch ticks did not strictly increase.
    EventTickNotIncreasing { incoming: u32, previous: u32 },
    /// An order was recorded with a sequence not above the last one.
    OrderSequence { incoming: u32, previous: u32 },
    /// Every order sequence number was used.
    SequenceExhausted,
    /// An order was issued before any snapshot.
    NoSnapshot,
    /// The server's match start disagreed with its welcome on the tick rate.
    TickRateMismatch { start: u16, welcome: u16 },
    /// The server's match start disagreed with its welcome on the tick mode.
    ModeMismatch { start: TickMode, welcome: TickMode },
    /// The match start named no pick for the assigned seat.
    NoPickForSeat(u8),
    /// The assigned seat picked a hero other than the one the client plays.
    WrongSeatHero {
        slot: u8,
        actual: HeroId,
        expected: HeroId,
    },
    /// The server broke the order of its stream; the text says how.
    Stream(&'static str),
}

/// A session input refused by the session or by the observer it called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestError<E> {
    Session(SessionError),
    Observer(E),
}

impl<E> From<SessionError> for IngestError<E> {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

impl From<IngestError<Infallible>> for SessionError {
    fn from(error: IngestError<Infallible>) -> Self {
        match error {
            IngestError::Session(error) => error,
            IngestError::Observer(never) => match never {},
        }
    }
}

impl<E: fmt::Display> fmt::Display for IngestError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(error) => error.fmt(formatter),
            Self::Observer(error) => error.fmt(formatter),
        }
    }
}

impl<E: Error + 'static> Error for IngestError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Session(error) => Some(error),
            Self::Observer(error) => Some(error),
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTickRate => formatter.write_str("MatchInfo.tick_rate must be positive"),
            Self::SeatCount(actual) => write!(
                formatter,
                "MatchInfo.picks has {actual} seats; expected 1..={MAX_SEATS}"
            ),
            Self::OwnSlotOutOfRange(slot) => write!(
                formatter,
                "own SlotId({slot}) is outside supported seats 0..{MAX_SEATS}"
            ),
            Self::MissingOwnPick(slot) => {
                write!(formatter, "MatchInfo.picks has no own SlotId({slot})")
            }
            Self::WrongOwnHero { actual, expected } => write!(
                formatter,
                "own slot picked HeroId({}), expected HeroId({})",
                actual.0, expected.0
            ),
            Self::InvalidOwnTeam(team) => {
                write!(formatter, "own slot has non-playable team {team:?}")
            }
            Self::DuplicatePickSlot(slot) => {
                write!(formatter, "MatchInfo.picks repeats SlotId({slot})")
            }
            Self::PickSlotOutOfRange(slot) => write!(
                formatter,
                "MatchInfo.picks contains unsupported SlotId({slot})"
            ),
            Self::TerrainAxis(cells) => write!(
                formatter,
                "MatchInfo.terrain_cells is {cells}; expected 1..={MAX_TERRAIN_AXIS}"
            ),
            Self::TerrainCoverage { actual, expected } => write!(
                formatter,
                "MatchInfo.terrain_rle covers {actual} cells; expected {expected}"
            ),
            Self::ZeroTerrainRun(index) => write!(
                formatter,
                "MatchInfo.terrain_rle entry {index} has zero run length"
            ),
            Self::OpaqueCellOutOfRange { x, y, cells } => write!(
                formatter,
                "MatchInfo.opaque_cells contains ({x}, {y}) outside {cells}x{cells} terrain"
            ),
            Self::PositionOutOfMap {
                field,
                index,
                x_raw,
                y_raw,
                extent,
            } => write!(
                formatter,
                "{field}[{index}] position raw ({x_raw}, {y_raw}) is outside {}..={}",
                extent.minimum(),
                extent.maximum()
            ),
            _ => self.fmt_snapshot(formatter),
        }
    }
}

impl SessionError {
    fn fmt_snapshot(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongViewer(actual, expected) => write!(
                formatter,
                "snapshot viewer {actual:?} differs from tracker team {expected:?}"
            ),
            Self::SnapshotTick { incoming, current } => write!(
                formatter,
                "snapshot tick {incoming} does not follow current tick {current}"
            ),
            Self::SkippedTicks { incoming } => write!(
                formatter,
                "snapshot tick {incoming} tells skipped ticks out of order or outside its gap"
            ),
            Self::MissingOwnPlayer(slot) => {
                write!(formatter, "snapshot players has no own SlotId({slot})")
            }
            Self::MissingOwnGold => {
                formatter.write_str("own player gold is absent in seat-specific snapshot")
            }
            Self::MissingOwnStash => {
                formatter.write_str("own player stash is absent in seat-specific snapshot")
            }
            Self::MissingOwnHero(id) => write!(
                formatter,
                "own live hero EntityId({}, {}) is absent from visible units",
                id.idx, id.generation
            ),
            Self::InvalidOwnHero(id) => write!(
                formatter,
                "own live hero EntityId({}, {}) has inconsistent kind, team, owner, or hero",
                id.idx, id.generation
            ),
            Self::AmbiguousOwnHero {
                scoreboard,
                visible,
            } => match scoreboard {
                Some(scoreboard) => write!(
                    formatter,
                    "own slot has visible hero EntityId({}, {}) besides scoreboard hero EntityId({}, {})",
                    visible.idx, visible.generation, scoreboard.idx, scoreboard.generation
                ),
                None => write!(
                    formatter,
                    "own slot has visible hero EntityId({}, {}) while scoreboard body is absent",
                    visible.idx, visible.generation
                ),
            },
            Self::AmbiguousOwnCourier { first, second } => write!(
                formatter,
                "own slot has ambiguous couriers EntityId({}, {}) and EntityId({}, {})",
                first.idx, first.generation, second.idx, second.generation
            ),
            _ => self.fmt_roster(formatter),
        }
    }

    fn fmt_roster(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateUnit(id) => write!(
                formatter,
                "snapshot units repeats EntityId({}, {})",
                id.idx, id.generation
            ),
            Self::DuplicateProjectile(id) => write!(
                formatter,
                "snapshot projectiles repeats EntityId({}, {})",
                id.idx, id.generation
            ),
            Self::DuplicateLoot(id) => write!(
                formatter,
                "snapshot loot repeats EntityId({}, {})",
                id.idx, id.generation
            ),
            Self::DuplicatePlayer(slot) => {
                write!(formatter, "snapshot players repeats SlotId({slot})")
            }
            Self::UnsortedPlayers => {
                formatter.write_str("snapshot players are not sorted by SlotId")
            }
            Self::UnsortedTalents => formatter.write_str(
                "snapshot talents are not strictly sorted by SlotId or name an unknown seat",
            ),
            Self::MissingRosterPlayer(slot) => write!(
                formatter,
                "snapshot players has no MatchInfo pick SlotId({slot})"
            ),
            Self::UnknownRosterPlayer(slot) => write!(
                formatter,
                "snapshot players contains unknown SlotId({slot})"
            ),
            Self::WrongRosterTeam {
                slot,
                actual,
                expected,
            } => write!(
                formatter,
                "snapshot player SlotId({slot}) team {actual:?} differs from MatchInfo pick {expected:?}"
            ),
            Self::WrongRosterHero {
                slot,
                actual,
                expected,
            } => write!(
                formatter,
                "snapshot player SlotId({slot}) HeroId({}) differs from MatchInfo pick HeroId({})",
                actual.0, expected.0
            ),
            _ => self.fmt_stream(formatter),
        }
    }

    fn fmt_stream(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RestorationReport { channel, amount } => write!(
                formatter,
                "Healed {channel} report {amount} is outside 0..={MAX_RESTORATION_REPORT}"
            ),
            Self::LimitExceeded {
                field,
                actual,
                limit,
            } => {
                write!(formatter, "{field} has {actual} entries; limit is {limit}")
            }
            Self::UnsortedUnits => {
                formatter.write_str("snapshot units are not sorted by full EntityId")
            }
            Self::UnsortedAggro => {
                formatter.write_str("snapshot aggro is not sorted strictly by unit EntityId")
            }
            Self::UnknownAggroUnit(id) => write!(
                formatter,
                "snapshot aggro names EntityId({}, {}), which is not a visible unit",
                id.idx, id.generation
            ),
            Self::AggroOutOfRange(id) => write!(
                formatter,
                "snapshot aggro of EntityId({}, {}) holds a timer out of range",
                id.idx, id.generation
            ),
            Self::InvalidFelledTree(index) => write!(
                formatter,
                "snapshot felled tree index {index} is outside static tree list"
            ),
            Self::PositionDeltaOverflow(id) => write!(
                formatter,
                "position delta for EntityId({}, {}) exceeds Vec2 range",
                id.idx, id.generation
            ),
            Self::EventTickNotIncreasing { incoming, previous } => write!(
                formatter,
                "event batch tick {incoming} must be greater than last event tick {previous}"
            ),
            _ => self.fmt_order(formatter),
        }
    }

    fn fmt_order(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrderSequence { incoming, previous } => write!(
                formatter,
                "order sequence {incoming} must be greater than last sent sequence {previous}"
            ),
            Self::SequenceExhausted => formatter.write_str("order sequence number exhausted"),
            Self::NoSnapshot => formatter.write_str("no snapshot has been received yet"),
            Self::TickRateMismatch { start, welcome } => write!(
                formatter,
                "MatchStart tick rate {start} differs from Welcome tick rate {welcome}"
            ),
            Self::ModeMismatch { start, welcome } => write!(
                formatter,
                "MatchStart mode {start:?} differs from Welcome mode {welcome:?}"
            ),
            Self::NoPickForSeat(slot) => write!(
                formatter,
                "MatchStart has no hero pick for assigned slot {slot}"
            ),
            Self::WrongSeatHero {
                slot,
                actual,
                expected,
            } => write!(
                formatter,
                "assigned slot {slot} picked HeroId({}), expected HeroId({})",
                actual.0, expected.0
            ),
            Self::Stream(text) => formatter.write_str(text),
            _ => unreachable!("every SessionError variant has a message"),
        }
    }
}

impl Error for SessionError {}
