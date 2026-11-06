//! Counts derived from the game clock, never accumulated across updates.
use crate::rules::Schedule;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrowthClock {
    pub first_after_horn: u32,
    pub period: u32,
    pub cap: u32,
}
impl GrowthClock {
    pub const fn count(self, clock: u32, schedule: Schedule) -> u32 {
        assert!(self.period > 0);
        let first = schedule.horn as u64 + self.first_after_horn as u64;
        if (clock as u64) < first {
            return 0;
        }
        let count = 1 + ((clock as u64) - first) / (self.period as u64);
        if count > self.cap as u64 {
            self.cap
        } else {
            count as u32
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralGrowth {
    pub clock: GrowthClock,
    pub ability: crate::NeutralAbility,
}

/// Damage added by one clock interval, per ability level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageGrowth {
    pub clock: GrowthClock,
    pub per_step: crate::Lv,
}
impl DamageGrowth {
    pub fn amount(self, base: i32, level: u8, clock: u32, schedule: Schedule) -> i32 {
        (i64::from(base)
            + i64::from(crate::at_level(&self.per_step, level))
                * i64::from(self.clock.count(clock, schedule)))
        .clamp(0, i32::MAX.into()) as i32
    }
}
