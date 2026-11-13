//! What can go wrong in the gym.

use core::fmt;

use bota_proto::RejectReason;

/// A refused configuration, a broken match contract, or a failed worker job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GymError {
    /// A parameter is outside its documented bounds; names the parameter and the bound.
    Config(&'static str),
    /// The simulator refused the match description.
    Match(String),
    /// A scenario's setup cheat was rejected by the match.
    SetupRejected {
        seat: usize,
        /// The cheat's index in [`MatchSpec::setup`](crate::MatchSpec::setup).
        index: usize,
        reason: RejectReason,
    },
    /// The match ended while its setup cheats were still being applied.
    SetupEnded,
    /// A step was given orders for a different number of seats than the match has.
    OrderCount { expected: usize, got: usize },
    /// A step was asked of a match that already ended.
    MatchOver,
    /// A step was asked of a match standing on its tick limit.
    TickLimit { tick: u32, limit: u32 },
    /// A seat's stream broke the contract a client relies on; a simulator bug.
    Stream { seat: usize, problem: &'static str },
    /// A seat's client session refused its stream or an order.
    Session { seat: usize, error: String },
    /// The match metadata could not be read back.
    Meta(String),
    /// An episode started or resumed as a different game than the driver asked for.
    GameMismatch,
    /// A driver was handed a different number of actions than it has episodes.
    ActionCount { expected: usize, got: usize },
    /// Caller code panicked on a worker while running the episode at this driver position
    /// (or the job at this index); the episode is gone.
    Panicked { position: usize },
    /// An earlier step of this driver failed and lost an episode; the driver cannot go on.
    Poisoned,
    /// The workers stopped before answering.
    Stopped,
    /// The workers kept [`MAX_UNDISPOSED_PANICS`](crate::workers::MAX_UNDISPOSED_PANICS) panic
    /// payloads they could not drop, and run no more jobs.
    Undisposed,
    /// A counter (game ordinal, random draws) would overflow.
    CounterOverflow,
}

impl fmt::Display for GymError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(what) => write!(formatter, "gym configuration refused: {what}"),
            Self::Match(message) => write!(formatter, "match setup refused: {message}"),
            Self::SetupRejected {
                seat,
                index,
                reason,
            } => write!(
                formatter,
                "setup cheat {index} of seat {seat} was rejected: {reason:?}"
            ),
            Self::SetupEnded => formatter.write_str("the match ended during its setup"),
            Self::OrderCount { expected, got } => write!(
                formatter,
                "orders for {got} seats given to a match of {expected} seats"
            ),
            Self::MatchOver => formatter.write_str("the match cannot step after MatchOver"),
            Self::TickLimit { tick, limit } => write!(
                formatter,
                "the match stands on tick {tick}, at its tick limit {limit}"
            ),
            Self::Stream { seat, problem } => {
                write!(
                    formatter,
                    "seat {seat}'s stream broke its contract: {problem}"
                )
            }
            Self::Session { seat, error } => {
                write!(formatter, "seat {seat}'s client session refused: {error}")
            }
            Self::Meta(message) => write!(formatter, "match metadata unreadable: {message}"),
            Self::GameMismatch => {
                formatter.write_str("an episode reported a different game than it was started as")
            }
            Self::ActionCount { expected, got } => write!(
                formatter,
                "{got} actions given to a driver of {expected} episodes"
            ),
            Self::Panicked { position } => {
                write!(formatter, "caller code panicked at position {position}")
            }
            Self::Poisoned => {
                formatter.write_str("the driver lost an episode to an earlier failure")
            }
            Self::Stopped => formatter.write_str("the gym workers stopped"),
            Self::Undisposed => formatter.write_str(
                "the workers kept too many panic payloads that panicked when dropped, and stopped",
            ),
            Self::CounterOverflow => formatter.write_str("a gym counter would overflow"),
        }
    }
}

impl std::error::Error for GymError {}
