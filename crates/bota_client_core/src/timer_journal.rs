//! A bounded journal of timers kept from the orders given, so that the timers
//! of a rejected order can be rolled back.

/// Recent timers kept per journal so a rejected order's timers can be rolled
/// back. An evicted timer becomes the base and can no longer be rejected; with
/// one order outstanding at a time that is never needed.
pub const MAX_TIMER_HISTORY: usize = 8;

/// The timers one order started, newest last, each with the sequence of the
/// order and a payload of the caller's (the tick it runs out, the tick it was
/// given on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerJournal<T: Copy> {
    base: Option<(u32, T)>,
    entries: [Option<(u32, T)>; MAX_TIMER_HISTORY],
    count: usize,
}

impl<T: Copy> TimerJournal<T> {
    pub const fn new() -> Self {
        Self {
            base: None,
            entries: [None; MAX_TIMER_HISTORY],
            count: 0,
        }
    }

    /// Records the timer the order with `sequence` started.
    pub fn push(&mut self, sequence: u32, value: T) {
        if self.count == MAX_TIMER_HISTORY {
            self.base = self.entries[0];
            self.entries.copy_within(1..MAX_TIMER_HISTORY, 0);
            self.count -= 1;
        }
        self.entries[self.count] = Some((sequence, value));
        self.count += 1;
    }

    /// Removes every timer the rejected order started; whether any was kept.
    pub fn reject(&mut self, sequence: u32) -> bool {
        let mut kept = [None; MAX_TIMER_HISTORY];
        let mut count = 0usize;
        let mut removed = false;
        for (at, value) in self.entries[..self.count].iter().flatten().copied() {
            if at == sequence {
                removed = true;
            } else {
                kept[count] = Some((at, value));
                count += 1;
            }
        }
        self.entries = kept;
        self.count = count;
        removed
    }

    /// The newest timer still standing.
    pub fn current(&self) -> Option<T> {
        self.entries[..self.count]
            .iter()
            .rev()
            .find_map(|timer| *timer)
            .or(self.base)
            .map(|(_, value)| value)
    }
}

impl<T: Copy> Default for TimerJournal<T> {
    fn default() -> Self {
        Self::new()
    }
}
