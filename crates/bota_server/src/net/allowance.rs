//! How many messages one connection may send.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Messages a connection may send by default every second, on top of
/// [`MESSAGES_PER_TICK`].
pub const DEFAULT_MESSAGES_PER_SEC: NonZeroU32 = NonZeroU32::new(1000).expect("nonzero");

/// Messages a connection may send for every tick it is sent, on top of its
/// rate: an acknowledgement and a few orders. A lockstep match runs as fast
/// as its seats answer, so the allowance keeps pace with the ticks, not with
/// the clock.
pub const MESSAGES_PER_TICK: u32 = 4;

/// How much of the rate a quiet connection may save up for a burst.
pub const BURST: Duration = Duration::from_secs(2);

/// The inbound allowance of one connection: a rate a second plus
/// [`MESSAGES_PER_TICK`] for every tick it was sent, with [`BURST`] of the
/// rate saved up at most.
///
/// Kept as a schedule rather than a count: `paid_until` is when every
/// message taken so far is paid for at the rate. A message moves it one
/// spacing on, a tick sent moves it back, and a message that would push it
/// more than the burst past now is refused.
pub struct Allowance {
    /// What one message costs of a second.
    spacing: Duration,
    paid_until: Instant,
    /// Ticks the connection was sent, counted by the server thread.
    sent: Arc<AtomicU32>,
    /// `sent` as it stood at the last message.
    credited: u32,
}

impl Allowance {
    /// A full allowance of `per_sec` a second, crediting the ticks counted in
    /// `sent`.
    pub fn new(per_sec: NonZeroU32, sent: Arc<AtomicU32>, now: Instant) -> Allowance {
        let credited = sent.load(Ordering::Relaxed);
        Allowance {
            spacing: Duration::from_secs(1) / per_sec.get(),
            paid_until: now,
            sent,
            credited,
        }
    }

    /// Takes one message from the allowance; false once it is spent.
    pub fn take(&mut self, now: Instant) -> bool {
        let sent = self.sent.load(Ordering::Relaxed);
        let ticks = sent.wrapping_sub(self.credited);
        self.credited = sent;
        let credit = self
            .spacing
            .saturating_mul(ticks.saturating_mul(MESSAGES_PER_TICK));
        let paid = self.paid_until.checked_sub(credit).unwrap_or(now).max(now);
        let next = paid + self.spacing;
        if next > now + BURST {
            self.paid_until = paid;
            return false;
        }
        self.paid_until = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arithmetic the E2E flood tests cannot isolate: the burst, the
    /// refill by the clock and the credit for ticks.
    #[test]
    fn a_burst_is_spent_then_the_rate_and_the_ticks_refill_it() {
        let start = Instant::now();
        let sent = Arc::new(AtomicU32::new(0));
        let rate = NonZeroU32::new(10).expect("nonzero");
        let mut allowance = Allowance::new(rate, Arc::clone(&sent), start);
        let burst = (0..100).take_while(|_| allowance.take(start)).count();
        assert_eq!(burst, 20, "two seconds' worth at ten a second");
        assert!(!allowance.take(start));
        assert!(allowance.take(start + Duration::from_millis(100)));
        assert!(!allowance.take(start + Duration::from_millis(100)));
        sent.fetch_add(3, Ordering::Relaxed);
        let later = start + Duration::from_millis(100);
        let credited = (0..100).take_while(|_| allowance.take(later)).count();
        assert_eq!(credited, 3 * MESSAGES_PER_TICK as usize);
    }
}
