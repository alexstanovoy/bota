//! How late the seat's orders take effect, as the server measures it.

use crate::session::ClientSession;

/// Most extra ticks a plan accounts for; a larger measurement is taken as this.
pub const MAX_PLANNED_ORDER_DELAY_TICKS: u32 = 15;
/// Extra ticks a plan accounts for before the server has measured any.
pub const ASSUMED_ORDER_DELAY_TICKS: u32 = 2;

/// The order delay one decision plans with: ticks an order given on the decision's snapshot
/// waits past the next tick before it takes effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct OrderLatency {
    delay: u32,
}

impl OrderLatency {
    /// Every order takes effect on the tick after its snapshot.
    pub const NONE: Self = Self { delay: 0 };

    /// A delay of `ticks`, at most [`MAX_PLANNED_ORDER_DELAY_TICKS`].
    pub const fn new(ticks: u32) -> Self {
        Self {
            delay: if ticks < MAX_PLANNED_ORDER_DELAY_TICKS {
                ticks
            } else {
                MAX_PLANNED_ORDER_DELAY_TICKS
            },
        }
    }

    /// The delay the session's current snapshot tells, or
    /// [`ASSUMED_ORDER_DELAY_TICKS`] before one was measured.
    pub fn measured(session: &ClientSession) -> Self {
        Self::new(session.order_delay().unwrap_or(ASSUMED_ORDER_DELAY_TICKS))
    }

    /// Ticks an order waits past the tick after its snapshot.
    pub const fn delay(self) -> u32 {
        self.delay
    }

    /// Ticks from a snapshot to the tick an order given on it takes effect.
    pub const fn lands(self) -> u32 {
        1 + self.delay
    }
}
