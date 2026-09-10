//! The one want a tick, and when it is worth saying again.
//!
//! The server keeps one order per seat per tick and the last one wins, so a
//! want is a single [`Ask`] rather than a queue. Saying the want already
//! standing is not free: an order ends the recovery after a swing and calls
//! the creeps onto whoever gave it. So a want equal to the one in hand waits
//! [`RESEND_TICKS`] before it goes out again, and two walks aimed less than
//! [`RESEND_DRIFT`] apart count as one want.

use bota_proto::{Order, Target};

use crate::{Ask, RESEND_DRIFT, RESEND_TICKS, span};

/// The want standing at the moment, and the tick it was last said on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Steady {
    /// What was last sent. Absent before anything has been.
    last: Option<Ask>,
    /// The tick it was sent on.
    since: u32,
}

impl Steady {
    /// A steady that has said nothing yet.
    pub fn new() -> Steady {
        Steady::default()
    }

    /// Whether an ask is worth putting on the wire this tick.
    ///
    /// Keeps whatever it lets through as the want now standing.
    pub fn worth_sending(&mut self, tick: u32, ask: Ask) -> bool {
        let standing = self.last.is_some_and(|had| {
            same_want(had, ask) && tick.saturating_sub(self.since) < RESEND_TICKS
        });
        if standing {
            return false;
        }
        self.last = Some(ask);
        self.since = tick;
        true
    }

    /// Forgets what was standing, so the next ask goes out whatever it is.
    pub fn forget(&mut self) {
        self.last = None;
    }
}

/// Whether two asks are the same want.
///
/// Two walks or two attack-moves aimed less than [`RESEND_DRIFT`] apart are
/// one want: a wave that moves a little every tick would otherwise be
/// followed by a fresh route every tick, and every route thrown away.
pub fn same_want(one: Ask, other: Ask) -> bool {
    if one.unit != other.unit {
        return false;
    }
    match (one.order, other.order) {
        (
            Order::Move {
                target: Target::Pos(here),
            },
            Order::Move {
                target: Target::Pos(there),
            },
        )
        | (
            Order::Attack {
                target: Target::Pos(here),
            },
            Order::Attack {
                target: Target::Pos(there),
            },
        ) => span(here, there) < RESEND_DRIFT,
        (here, there) => here == there,
    }
}
