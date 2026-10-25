//! The clock before the match, as Dota keeps it: a selection time, then a
//! penalty phase in which a seat still without a hero loses starting gold
//! every second until it picks or is given a hero, then the tactical
//! phase.

use std::time::{Duration, Instant};

use bota_proto::PickPhase;

/// Where the selection stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickClock {
    /// Not every seat has been held yet; the time has not started.
    Waiting,
    /// Seats pick until `ends`.
    Selecting { ends: Instant },
    /// The time ran out at `since`; seats without a hero lose gold until
    /// `ends`, when each is given one.
    Late { since: Instant, ends: Instant },
    /// Every pick is final; the match begins at `ends`.
    Tactical { ends: Instant },
}

impl PickClock {
    /// The clock when every seat has just been held.
    pub fn start(now: Instant, time: Duration) -> PickClock {
        PickClock::Selecting { ends: now + time }
    }

    /// The clock once the selection time is up.
    pub fn run_out(ends: Instant, penalty: Duration) -> PickClock {
        PickClock::Late {
            since: ends,
            ends: ends + penalty,
        }
    }

    /// When the clock next changes by itself.
    pub fn due(&self) -> Option<Instant> {
        match self {
            PickClock::Selecting { ends }
            | PickClock::Late { ends, .. }
            | PickClock::Tactical { ends } => Some(*ends),
            PickClock::Waiting => None,
        }
    }

    /// Whole seconds past the selection time at `now`, at most the penalty
    /// phase; nothing outside it.
    pub fn late_secs(&self, now: Instant) -> u32 {
        match self {
            PickClock::Late { since, ends } => {
                now.min(*ends).saturating_duration_since(*since).as_secs() as u32
            }
            _ => 0,
        }
    }

    /// Whether picks are final the moment they are made.
    pub fn is_past_selection(&self) -> bool {
        !matches!(self, PickClock::Waiting | PickClock::Selecting { .. })
    }

    /// What the wire is told.
    pub fn phase(&self, now: Instant) -> PickPhase {
        let left = |ends: &Instant| {
            u32::try_from(ends.saturating_duration_since(now).as_millis()).unwrap_or(u32::MAX)
        };
        match self {
            PickClock::Waiting => PickPhase::Waiting,
            PickClock::Selecting { ends } => PickPhase::Selecting {
                left_ms: left(ends),
            },
            PickClock::Late { ends, .. } => PickPhase::Late {
                left_ms: left(ends),
            },
            PickClock::Tactical { ends } => PickPhase::Tactical {
                left_ms: left(ends),
            },
        }
    }
}
