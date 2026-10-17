//! Day and night, read off the tick alone.
//!
//! Dota's cycle (Liquipedia, Time of Day): the pregame is night, the first
//! day breaks at the horn, and day and night then take turns of five minutes
//! each. Every unit sees by its day or its night vision accordingly, and
//! neutral camps sleep through the night.

use crate::rules;

/// Whether it is night on a tick, and for how much longer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimeOfDay {
    pub night: bool,
    /// Ticks until it turns, at least 1: tick `tick + ticks_left` is the
    /// first of the other.
    pub ticks_left: u32,
}

/// The time of day on a tick.
///
/// A pure function of the tick, the same in every match and for every
/// viewer, so it is no state of the world and crosses no wire: a client and
/// a replay read it off a view's tick.
pub const fn time_of_day(tick: u32) -> TimeOfDay {
    let Some(since_horn) = tick.checked_sub(rules::PREGAME_TICKS) else {
        return TimeOfDay {
            night: true,
            ticks_left: rules::PREGAME_TICKS - tick,
        };
    };
    TimeOfDay {
        night: (since_horn / rules::HALF_DAY_TICKS) % 2 == 1,
        ticks_left: rules::HALF_DAY_TICKS - since_horn % rules::HALF_DAY_TICKS,
    }
}
