//! Why a session refused its input.

use core::convert::Infallible;
use core::fmt;
use std::error::Error;

use bota_proto::{EntityId, HeroId, RejectReason, Team, TickMode, WIRE_FINGERPRINT};

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
    /// The shop repeats an item, names a part it does not sell, or builds an
    /// item from itself.
    InvalidShop(&'static str),
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
    /// The server was built with another wire and refused the connection.
    WireMismatch {
        /// The server's [`WIRE_FINGERPRINT`].
        server: u64,
    },
    /// The server refused the connection's `Hello`.
    Refused(RejectReason),
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTickRate => f.write_str("MatchInfo.tick_rate must be positive"),
            Self::SeatCount(n) => {
                write!(f, "MatchInfo.picks has {n} seats; expected 1..={MAX_SEATS}")
            }
            Self::OwnSlotOutOfRange(slot) => {
                write!(
                    f,
                    "own SlotId({slot}) is outside supported seats 0..{MAX_SEATS}"
                )
            }
            Self::MissingOwnPick(slot) => write!(f, "MatchInfo.picks has no own SlotId({slot})"),
            Self::WrongOwnHero { actual, expected } => write!(
                f,
                "own slot picked {}, expected {}",
                Named(*actual),
                Named(*expected)
            ),
            Self::InvalidOwnTeam(team) => write!(f, "own slot has non-playable team {team:?}"),
            Self::DuplicatePickSlot(slot) => write!(f, "MatchInfo.picks repeats SlotId({slot})"),
            Self::PickSlotOutOfRange(slot) => {
                write!(f, "MatchInfo.picks contains unsupported SlotId({slot})")
            }
            Self::TerrainAxis(cells) => write!(
                f,
                "MatchInfo.terrain_cells is {cells}; expected 1..={MAX_TERRAIN_AXIS}"
            ),
            Self::TerrainCoverage { actual, expected } => write!(
                f,
                "MatchInfo.terrain_rle covers {actual} cells; expected {expected}"
            ),
            Self::ZeroTerrainRun(index) => {
                write!(f, "MatchInfo.terrain_rle entry {index} has zero run length")
            }
            Self::InvalidShop(why) => write!(f, "MatchInfo.shop {why}"),
            Self::OpaqueCellOutOfRange { x, y, cells } => write!(
                f,
                "MatchInfo.opaque_cells contains ({x}, {y}) outside {cells}x{cells} terrain"
            ),
            Self::PositionOutOfMap {
                field,
                index,
                x_raw,
                y_raw,
                extent,
            } => write!(
                f,
                "{field}[{index}] position raw ({x_raw}, {y_raw}) is outside {}..={}",
                extent.minimum(),
                extent.maximum()
            ),
            _ => self.fmt_snapshot(f),
        }
    }
}

impl SessionError {
    fn fmt_snapshot(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongViewer(actual, expected) => write!(
                f,
                "snapshot viewer {actual:?} differs from tracker team {expected:?}"
            ),
            Self::SnapshotTick { incoming, current } => write!(
                f,
                "snapshot tick {incoming} does not follow current tick {current}"
            ),
            Self::SkippedTicks { incoming } => write!(
                f,
                "snapshot tick {incoming} tells skipped ticks out of order or outside its gap"
            ),
            Self::MissingOwnPlayer(slot) => write!(f, "snapshot players has no own SlotId({slot})"),
            Self::MissingOwnGold => {
                f.write_str("own player gold is absent in seat-specific snapshot")
            }
            Self::MissingOwnStash => {
                f.write_str("own player stash is absent in seat-specific snapshot")
            }
            Self::MissingOwnHero(id) => {
                write!(f, "own live hero {} is absent from visible units", Id(*id))
            }
            Self::InvalidOwnHero(id) => write!(
                f,
                "own live hero {} has inconsistent kind, team, owner, or hero",
                Id(*id)
            ),
            Self::AmbiguousOwnHero {
                scoreboard: Some(scoreboard),
                visible,
            } => write!(
                f,
                "own slot has visible hero {} besides scoreboard hero {}",
                Id(*visible),
                Id(*scoreboard)
            ),
            Self::AmbiguousOwnHero {
                scoreboard: None,
                visible,
            } => write!(
                f,
                "own slot has visible hero {} while scoreboard body is absent",
                Id(*visible)
            ),
            Self::AmbiguousOwnCourier { first, second } => write!(
                f,
                "own slot has ambiguous couriers {} and {}",
                Id(*first),
                Id(*second)
            ),
            _ => self.fmt_roster(f),
        }
    }

    fn fmt_roster(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateUnit(id) => write!(f, "snapshot units repeats {}", Id(*id)),
            Self::DuplicateProjectile(id) => write!(f, "snapshot projectiles repeats {}", Id(*id)),
            Self::DuplicateLoot(id) => write!(f, "snapshot loot repeats {}", Id(*id)),
            Self::DuplicatePlayer(slot) => write!(f, "snapshot players repeats SlotId({slot})"),
            Self::UnsortedPlayers => f.write_str("snapshot players are not sorted by SlotId"),
            Self::UnsortedTalents => f.write_str(
                "snapshot talents are not strictly sorted by SlotId or name an unknown seat",
            ),
            Self::MissingRosterPlayer(slot) => {
                write!(f, "snapshot players has no MatchInfo pick SlotId({slot})")
            }
            Self::UnknownRosterPlayer(slot) => {
                write!(f, "snapshot players contains unknown SlotId({slot})")
            }
            Self::WrongRosterTeam {
                slot,
                actual,
                expected,
            } => write!(
                f,
                "snapshot player SlotId({slot}) team {actual:?} differs from MatchInfo pick {expected:?}"
            ),
            Self::WrongRosterHero {
                slot,
                actual,
                expected,
            } => write!(
                f,
                "snapshot player SlotId({slot}) plays {}, differing from MatchInfo pick {}",
                Named(*actual),
                Named(*expected)
            ),
            _ => self.fmt_stream(f),
        }
    }

    fn fmt_stream(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RestorationReport { channel, amount } => write!(
                f,
                "Healed {channel} report {amount} is outside 0..={MAX_RESTORATION_REPORT}"
            ),
            Self::LimitExceeded {
                field,
                actual,
                limit,
            } => {
                write!(f, "{field} has {actual} entries; limit is {limit}")
            }
            Self::UnsortedUnits => f.write_str("snapshot units are not sorted by full EntityId"),
            Self::UnsortedAggro => {
                f.write_str("snapshot aggro is not sorted strictly by unit EntityId")
            }
            Self::UnknownAggroUnit(id) => write!(
                f,
                "snapshot aggro names {}, which is not a visible unit",
                Id(*id)
            ),
            Self::AggroOutOfRange(id) => {
                write!(
                    f,
                    "snapshot aggro of {} holds a timer out of range",
                    Id(*id)
                )
            }
            Self::InvalidFelledTree(index) => write!(
                f,
                "snapshot felled tree index {index} is outside static tree list"
            ),
            Self::PositionDeltaOverflow(id) => {
                write!(f, "position delta for {} exceeds Vec2 range", Id(*id))
            }
            Self::EventTickNotIncreasing { incoming, previous } => write!(
                f,
                "event batch tick {incoming} must be greater than last event tick {previous}"
            ),
            _ => self.fmt_order(f),
        }
    }

    fn fmt_order(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrderSequence { incoming, previous } => write!(
                f,
                "order sequence {incoming} must be greater than last sent sequence {previous}"
            ),
            Self::SequenceExhausted => f.write_str("order sequence number exhausted"),
            Self::NoSnapshot => f.write_str("no snapshot has been received yet"),
            Self::TickRateMismatch { start, welcome } => write!(
                f,
                "MatchStart tick rate {start} differs from Welcome tick rate {welcome}"
            ),
            Self::ModeMismatch { start, welcome } => write!(
                f,
                "MatchStart mode {start:?} differs from Welcome mode {welcome:?}"
            ),
            Self::NoPickForSeat(slot) => {
                write!(f, "MatchStart has no hero pick for assigned slot {slot}")
            }
            Self::WrongSeatHero {
                slot,
                actual,
                expected,
            } => write!(
                f,
                "assigned slot {slot} picked {}, expected {}",
                Named(*actual),
                Named(*expected)
            ),
            Self::Stream(text) => f.write_str(text),
            Self::WireMismatch { server } => write!(
                f,
                "the server speaks wire {server:016x} and this client {WIRE_FINGERPRINT:016x}; \
                 build both from matching commits"
            ),
            Self::Refused(reason) => write!(
                f,
                "the server refused this client: {}",
                crate::orders::refusal(*reason)
            ),
            _ => unreachable!("every SessionError variant has a message"),
        }
    }
}

impl Error for SessionError {}

/// An entity handle as the messages print it.
struct Id(EntityId);

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EntityId({}, {})", self.0.idx, self.0.generation)
    }
}

/// A hero by its name, or by its id when this build does not know it.
struct Named(HeroId);

impl fmt::Display for Named {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match bota_core::hero_def(self.0) {
            Some(hero) => formatter.write_str(hero.name),
            None => write!(formatter, "HeroId({})", self.0.0),
        }
    }
}
