//! What the service that runs the server asks of it from outside the match: end it now, and end
//! it at a time limit, with a warning ahead.
//!
//! The server never reacts to either where it hears of it: it looks at [`Outside`] between its own
//! steps (a lobby wait, a tick) and ends the match from there, so the end is announced, the
//! replay closed and the result printed by the code that ends every match.
//!
//! The time limit counts the wall-clock time of the service that runs the server, from the server's
//! start (just before it says `READY`, so a launcher that counts from `READY` sees the game's own
//! limit fall due first): not game time, which stops with a stuck game, and not ticks, which a
//! lockstep match spends as fast as the machine allows. The same in lockstep and in realtime.
//! The clock is behind [`LimitClock`], so that a test moves it by hand instead of waiting.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bota_proto::AbortReason;

/// How long before the limit the players are warned, unless the limit is so short that half of it
/// is less.
pub const DEFAULT_WARNING: Duration = Duration::from_secs(5 * 60);

/// How often a server that is asked to watch looks, at the most: a signal wakes nothing.
const LOOK_EVERY: Duration = Duration::from_millis(250);

/// How often a server on a hand-moved clock looks, at the most: a test does not wait for it.
const LOOK_MANUAL: Duration = Duration::from_millis(5);

/// The clock the time limit counts on: the wall clock from the creation of [`Outside`] (the
/// binary makes it before it says `READY`), or a hand-moved one for tests.
#[derive(Clone, Debug)]
pub enum LimitClock {
    /// Real time since the instant.
    Wall(Instant),
    /// Time only passes when [`ManualClock::set`] says so.
    Manual(ManualClock),
}

impl LimitClock {
    fn elapsed(&self) -> Duration {
        match self {
            Self::Wall(origin) => origin.elapsed(),
            Self::Manual(clock) => clock.elapsed(),
        }
    }

    fn is_wall(&self) -> bool {
        matches!(self, Self::Wall(_))
    }
}

/// A clock a test moves: the time the limit has run, in milliseconds, shared by every clone.
#[derive(Clone, Debug, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    /// A clock at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the clock to `elapsed` (it may go backwards: nothing in the server needs it not to).
    pub fn set(&self, elapsed: Duration) {
        let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        self.0.store(millis, Ordering::SeqCst);
    }

    fn elapsed(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

/// The time a match may last, counted on a [`LimitClock`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeLimit {
    total: Duration,
    warn_before: Duration,
}

impl TimeLimit {
    /// A limit of `total`, warned `warn_before` it (at most half of it, so that the warning never
    /// comes before the match has been played for as long as it still has to be). Nothing for a
    /// limit of no time.
    #[must_use]
    pub fn new(total: Duration, warn_before: Duration) -> Option<Self> {
        (!total.is_zero()).then(|| Self {
            total,
            warn_before: warn_before.min(total / 2),
        })
    }

    /// The whole limit.
    #[must_use]
    pub fn total(self) -> Duration {
        self.total
    }

    /// The warning's lead, as it will be sent.
    #[must_use]
    pub fn warn_before(self) -> Duration {
        self.warn_before
    }
}

/// A flag the service sets to ask the server to end the match now: a signal handler's (see
/// `signal_hook::flag`), or a test's own.
#[derive(Clone, Debug, Default)]
pub struct StopRequest(Option<Arc<AtomicBool>>);

impl StopRequest {
    /// A request that is never made.
    pub const NEVER: Self = Self(None);

    /// A request made by setting `flag`.
    #[must_use]
    pub fn on(flag: Arc<AtomicBool>) -> Self {
        Self(Some(flag))
    }

    fn made(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }
}

/// What the outside asks at one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Nothing.
    Nothing,
    /// Tell everybody that the limit is `remaining` away. Asked once.
    Warn { remaining: Duration },
    /// End the match now.
    End(AbortReason),
}

/// The outside world's say over one server, looked at as it runs.
#[derive(Clone, Debug)]
pub struct Outside {
    /// The time limit, if the service gave one.
    pub limit: Option<TimeLimit>,
    /// The request to end the match now.
    pub stop: StopRequest,
    clock: LimitClock,
    warned: bool,
}

impl Default for Outside {
    fn default() -> Self {
        Self::new(None, StopRequest::NEVER)
    }
}

impl Outside {
    /// Watches a limit and a request, the limit counted from now.
    #[must_use]
    pub fn new(limit: Option<TimeLimit>, stop: StopRequest) -> Self {
        Self::on_clock(limit, stop, LimitClock::Wall(Instant::now()))
    }

    /// Watches a limit and a request, the limit counted on `clock`.
    #[must_use]
    pub fn on_clock(limit: Option<TimeLimit>, stop: StopRequest, clock: LimitClock) -> Self {
        Self {
            limit,
            stop,
            clock,
            warned: false,
        }
    }

    /// The time since the limit's clock started.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.clock.elapsed()
    }

    /// What a connection that is admitted after the warning went out is told: the time that is
    /// left. Nothing before the warning, which reaches every connection then.
    #[must_use]
    pub fn warned_remaining(&self) -> Option<Duration> {
        let limit = self.limit.filter(|_| self.warned)?;
        Some(limit.total.saturating_sub(self.elapsed()))
    }

    /// What is asked of the server when it has run for `elapsed`. A request to stop wins over the
    /// limit; the warning is asked once, the first time it is due, and never when the limit has
    /// already run out.
    pub fn ask(&mut self, elapsed: Duration) -> Ask {
        if self.stop.made() {
            return Ask::End(AbortReason::Stopped);
        }
        let Some(limit) = self.limit else {
            return Ask::Nothing;
        };
        if elapsed >= limit.total {
            return Ask::End(AbortReason::TimeLimit);
        }
        if !self.warned && elapsed + limit.warn_before >= limit.total {
            self.warned = true;
            return Ask::Warn {
                remaining: limit.total - elapsed,
            };
        }
        Ask::Nothing
    }

    /// How long a server that has run for `elapsed` may sleep before it must look again: not
    /// at all when nothing is watched.
    #[must_use]
    pub fn look_again_in(&self, elapsed: Duration) -> Option<Duration> {
        if self.limit.is_none() && self.stop.0.is_none() {
            return None;
        }
        let due = self.limit.map(|limit| {
            let warning_at = limit.total - limit.warn_before;
            let next = if self.warned { limit.total } else { warning_at };
            next.saturating_sub(elapsed)
        });
        // A clock that is moved by hand has no due time to sleep to: it is looked at often.
        let every = if self.clock.is_wall() {
            LOOK_EVERY
        } else {
            LOOK_MANUAL
        };
        Some(due.map_or(every, |due| due.min(every)))
    }
}

/// Whole seconds of `remaining` for the wire, a started second counted: never less than one.
#[must_use]
pub fn wire_seconds(remaining: Duration) -> u32 {
    let whole = remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
    u32::try_from(whole).unwrap_or(u32::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn limited(total: u64, warn: u64) -> Outside {
        Outside::new(TimeLimit::new(secs(total), secs(warn)), StopRequest::NEVER)
    }

    #[test]
    fn a_warning_comes_once_ahead_of_the_limit_and_the_limit_ends_the_match() {
        let mut watch = limited(600, 300);
        assert_eq!(watch.ask(secs(299)), Ask::Nothing);
        assert_eq!(
            watch.ask(secs(300)),
            Ask::Warn {
                remaining: secs(300)
            }
        );
        assert_eq!(watch.ask(secs(301)), Ask::Nothing, "asked once");
        assert_eq!(watch.ask(secs(600)), Ask::End(AbortReason::TimeLimit));
    }

    #[test]
    fn a_limit_shorter_than_the_warning_warns_at_its_half() {
        let limit = TimeLimit::new(secs(60), secs(300)).expect("a limit");
        assert_eq!(limit.warn_before(), secs(30));
    }

    #[test]
    fn a_limit_passed_without_a_look_ends_the_match_without_a_late_warning() {
        let mut watch = limited(600, 300);
        assert_eq!(watch.ask(secs(700)), Ask::End(AbortReason::TimeLimit));
    }

    #[test]
    fn a_request_to_stop_ends_the_match_whatever_the_clock_says() {
        let flag = Arc::new(AtomicBool::new(false));
        let mut watch = Outside::new(None, StopRequest::on(Arc::clone(&flag)));
        assert_eq!(watch.ask(secs(1)), Ask::Nothing);
        flag.store(true, Ordering::Relaxed);
        assert_eq!(watch.ask(secs(1)), Ask::End(AbortReason::Stopped));
    }

    #[test]
    fn only_a_warning_that_went_out_is_caught_up_with() {
        let mut watch = limited(600, 300);
        assert_eq!(watch.warned_remaining(), None, "nothing was said yet");
        let _ = watch.ask(secs(300));
        assert!(
            watch
                .warned_remaining()
                .is_some_and(|left| left <= secs(600))
        );
        assert_eq!(Outside::default().warned_remaining(), None);
    }

    #[test]
    fn a_hand_moved_clock_is_the_time_of_the_limit_and_is_looked_at_often() {
        let clock = ManualClock::new();
        let limit = TimeLimit::new(secs(600), secs(300));
        let mut watch =
            Outside::on_clock(limit, StopRequest::NEVER, LimitClock::Manual(clock.clone()));
        assert_eq!(watch.elapsed(), Duration::ZERO, "no time passes by itself");
        assert_eq!(watch.ask(watch.elapsed()), Ask::Nothing);
        clock.set(secs(600));
        assert_eq!(watch.elapsed(), secs(600));
        assert_eq!(watch.ask(watch.elapsed()), Ask::End(AbortReason::TimeLimit));
        assert_eq!(watch.look_again_in(Duration::ZERO), Some(LOOK_MANUAL));
    }

    #[test]
    fn nothing_watched_means_nothing_to_look_for() {
        let watch = Outside::default();
        assert_eq!(watch.look_again_in(secs(5)), None);
    }

    #[test]
    fn a_watched_limit_is_looked_at_at_the_latest_when_it_falls_due() {
        let mut watch = limited(600, 300);
        assert_eq!(watch.look_again_in(secs(100)), Some(LOOK_EVERY));
        assert_eq!(
            watch.look_again_in(Duration::from_millis(299_900)),
            Some(Duration::from_millis(100))
        );
        let _ = watch.ask(secs(300));
        assert_eq!(
            watch.look_again_in(Duration::from_millis(599_950)),
            Some(Duration::from_millis(50))
        );
    }

    #[test]
    fn seconds_on_the_wire_count_a_started_second_and_never_say_zero() {
        assert_eq!(wire_seconds(Duration::from_millis(1)), 1);
        assert_eq!(wire_seconds(Duration::from_millis(2_001)), 3);
        assert_eq!(wire_seconds(secs(300)), 300);
        assert_eq!(wire_seconds(Duration::ZERO), 1);
    }

    #[test]
    fn a_limit_of_no_time_is_none() {
        assert_eq!(TimeLimit::new(Duration::ZERO, secs(1)), None);
    }
}
