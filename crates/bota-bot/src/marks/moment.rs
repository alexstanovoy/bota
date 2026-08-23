//! One tick, as a lesson is shown it, and what a lesson carries between ticks.
//!
//! Everything a lesson could be paid for is here: where the hero stands, what
//! it can see, what happened during the tick, and what the scoreboard moved by.
//! A lesson is handed one of these and answers with one number, so a lesson
//! never reaches back into the wire or the match loop.

use bota_proto::{EventKind, Vec2};

use crate::{Field, Lane};

/// One tick, with everything that happened during it.
pub struct Moment<'a> {
    /// The tick, read into a settled shape.
    pub field: &'a Field<'a>,
    /// The lane this seat's role and side is held to. Absent while a fountain
    /// is out of sight, which is when a lane cannot be worked out at all.
    pub lane: Option<&'a Lane>,
    /// What happened during the tick.
    pub events: &'a [EventKind],
    /// Their creeps this seat landed the last hit on during the tick.
    pub took: u16,
    /// Them it killed during the tick.
    pub killed: u16,
    /// Times it died during the tick.
    pub died: u16,
}

impl Moment<'_> {
    /// Which tick it is.
    pub fn tick(&self) -> u32 {
        self.field.view.tick
    }

    /// Where the hero stands.
    pub fn at(&self) -> Vec2 {
        self.field.at()
    }

    /// Whether there is a body standing at all.
    pub fn alive(&self) -> bool {
        self.field.alive()
    }
}

/// What one lesson carries from one tick to the next.
///
/// One of these a lesson, so what a lesson remembers is its own: the spot two
/// lessons walk towards is not the same spot, and how far off it was last tick
/// means nothing to the other.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Carried {
    /// How far off the spot this lesson wants the hero was last tick.
    pub was_off: Option<f32>,
    /// What the seat's goods were worth last tick.
    pub was_owned: Option<i32>,
    /// What the seat was worth last tick, counting the purse at less than its
    /// face value.
    pub was_worth: Option<f32>,
    /// How many of their towers have come down.
    pub towers_down: u16,
}
