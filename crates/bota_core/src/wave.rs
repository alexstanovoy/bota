//! What a creep wave is made of, and how it grows with the clock.
//!
//! The composition follows the installed Dota 7.42 wave rules.

use bota_proto::Vec2;

use crate::isqrt64;
use crate::rules::{self, Schedule};
use crate::wave_data;

/// The make-up of one wave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavePlan {
    /// Melee creeps, the flagbearer among them.
    pub melee: u32,
    /// Whether one of the melee creeps carries the flag.
    pub flagbearer: bool,
    /// Ranged creeps.
    pub ranged: u32,
    /// Siege creeps.
    pub siege: u32,
    /// Elapsed upgrade intervals. Combat growth is capped when applied;
    /// bounties continue growing. Siege creeps gain nothing from them.
    pub upgrades: u32,
}

impl WavePlan {
    /// Creeps in the wave, all kinds counted.
    pub fn size(&self) -> u32 {
        self.melee + self.ranged + self.siege
    }
}

/// The wave due at a tick, counting from one, if a wave is due at all.
#[inline]
pub fn wave_at(tick: u32, schedule: Schedule) -> Option<u32> {
    latest_wave(tick, schedule)
        .filter(|_| (tick - schedule.first_wave()).is_multiple_of(rules::WAVE_PERIOD_TICKS))
}

/// The latest wave due at or before a tick, counting from one; none before
/// the first.
#[inline]
pub const fn latest_wave(tick: u32, schedule: Schedule) -> Option<u32> {
    if tick < schedule.first_wave() {
        return None;
    }
    Some((tick - schedule.first_wave()) / rules::WAVE_PERIOD_TICKS + 1)
}

/// The tick the given wave, counting from one, walks out at.
#[inline]
pub const fn wave_spawn_tick(wave: u32, schedule: Schedule) -> u32 {
    schedule.first_wave() + (wave - 1) * rules::WAVE_PERIOD_TICKS
}

/// Upgrades a wave's creeps carry: one per [`rules::WAVE_UPGRADE_PERIOD`]
/// waves. Combat growth alone is capped at [`rules::WAVE_UPGRADE_CAP`].
pub fn wave_upgrades(wave: u32) -> u32 {
    (wave - 1) / rules::WAVE_UPGRADE_PERIOD
}

/// What the given wave is made of.
pub fn wave_plan(wave: u32) -> WavePlan {
    let seconds = (wave - 1).saturating_mul(rules::WAVE_PERIOD_TICKS / rules::TICKS_PER_SECOND);
    let melee = rules::MELEE_PER_WAVE
        + rules::MELEE_GROWTH
            .iter()
            .filter(|&&from| {
                from <= wave
                    && (from - 1) * (rules::WAVE_PERIOD_TICKS / rules::TICKS_PER_SECOND)
                        != wave_data::OMITTED_MELEE_GROWTH_SECONDS
            })
            .count() as u32;
    let ranged = rules::RANGED_PER_WAVE + u32::from(seconds >= wave_data::RANGED_GROWTH_SECONDS);
    let (_, siege_interval, siege_count) = wave_data::SIEGE
        .into_iter()
        .rev()
        .find(|&(from, _, _)| seconds >= from)
        .expect("siege schedule starts at the horn");
    let siege = if seconds != 0 && seconds.is_multiple_of(siege_interval) {
        siege_count
    } else {
        0
    };
    let flagbearer = wave >= rules::FIRST_FLAGBEARER_WAVE
        && (seconds >= wave_data::FLAG_EVERY_WAVE_SECONDS
            || (wave - rules::FIRST_FLAGBEARER_WAVE).is_multiple_of(rules::FLAGBEARER_WAVE_PERIOD));
    WavePlan {
        melee,
        flagbearer,
        ranged,
        siege,
        upgrades: wave_upgrades(wave),
    }
}

/// Where each creep of a wave stands when it spawns.
///
/// Melee abreast across the march, siege among them, ranged a rank behind.
/// `forward` is the direction the wave marches.
pub fn spawn_offsets(plan: &WavePlan, forward: Vec2) -> Vec<Vec2> {
    let len = isqrt64(forward.length_squared());
    // A wave with nowhere to march lines up along the x axis.
    let (dx, dy, len) = if len == 0 {
        (1i64, 0i64, 1i64)
    } else {
        (i64::from(forward.x.raw), i64::from(forward.y.raw), len)
    };
    let along = |n: i32| {
        Vec2::from_ints(
            (dx * i64::from(n) / len) as i32,
            (dy * i64::from(n) / len) as i32,
        )
    };
    let across = |n: i32| {
        Vec2::from_ints(
            (-dy * i64::from(n) / len) as i32,
            (dx * i64::from(n) / len) as i32,
        )
    };
    let mut out = Vec::with_capacity(plan.size() as usize);
    let front = plan.melee + plan.siege;
    let spread = |i: u32, of: u32| (i as i32 - (of as i32 - 1) / 2) * rules::WAVE_SPAWN_SPACING;
    for i in 0..front {
        out.push(across(spread(i, front)));
    }
    for i in 0..plan.ranged {
        out.push(across(spread(i, plan.ranged)) + along(-rules::WAVE_SPAWN_RANK));
    }
    out
}

/// Whether the camps fill at a tick: on the minute mark from the first one.
#[inline]
pub fn neutral_spawn_due(tick: u32, schedule: Schedule) -> bool {
    tick >= schedule.first_neutral()
        && (tick - schedule.first_neutral()).is_multiple_of(rules::NEUTRAL_SPAWN_PERIOD_TICKS)
}

/// How many neutral spawns have happened at or before a tick.
#[inline]
pub const fn neutral_spawns_through(tick: u32, schedule: Schedule) -> u32 {
    if tick < schedule.first_neutral() {
        0
    } else {
        (tick - schedule.first_neutral()) / rules::NEUTRAL_SPAWN_PERIOD_TICKS + 1
    }
}

/// The first neutral spawn after a tick.
#[inline]
pub const fn next_neutral_spawn(tick: u32, schedule: Schedule) -> u32 {
    schedule.first_neutral()
        + neutral_spawns_through(tick, schedule) * rules::NEUTRAL_SPAWN_PERIOD_TICKS
}

/// How many upgrade intervals the jungle carries by a tick, capped.
#[inline]
pub fn jungle_upgrades(tick: u32, schedule: Schedule) -> u32 {
    let since = tick.saturating_sub(schedule.horn);
    (since / rules::NEUTRAL_UPGRADE_PERIOD_TICKS).min(rules::NEUTRAL_UPGRADE_CAP as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_wave_is_the_last_one_due() {
        for map in crate::MAPS {
            let schedule = map.schedule();
            let first = schedule.first_wave();
            assert_eq!(latest_wave(first - 1, schedule), None);
            for tick in first..first + 5 * rules::WAVE_PERIOD_TICKS {
                let latest = latest_wave(tick, schedule).unwrap();
                let due = (first..=tick).rev().find_map(|t| wave_at(t, schedule));
                assert_eq!(Some(latest), due);
            }
        }
    }
}

#[cfg(test)]
mod patch_742_tests {
    use super::*;

    // “Number of melee creeps spawned no longer increases at 30:00”.
    #[test]
    fn melee_waves_gain_creeps_at_fifteen_and_forty_five_minutes() {
        for (wave, count) in [(30, 3), (31, 4), (61, 4), (90, 4), (91, 5)] {
            assert_eq!(wave_plan(wave).melee, count, "wave {wave}");
        }
    }

    // “Time at which 1 Ranged creep is added changed from 40:00 to 35:00”.
    #[test]
    fn ranged_waves_gain_a_creep_at_thirty_five_minutes() {
        assert_eq!(wave_plan(70).ranged, 1);
        assert_eq!(wave_plan(71).ranged, 2);
    }

    // “Reworked Siege creep spawn count after the 30:00 mark”.
    #[test]
    fn siege_waves_double_in_frequency_then_in_size() {
        for (wave, count) in [
            (1, 0),
            (11, 1),
            (56, 0),
            (61, 1),
            (65, 0),
            (66, 1),
            (116, 1),
            (120, 0),
            (121, 2),
            (126, 2),
        ] {
            assert_eq!(wave_plan(wave).siege, count, "wave {wave}");
        }
    }

    // “Flagbearer creep spawn every wave after the 30:00 mark”.
    #[test]
    fn every_wave_after_thirty_minutes_has_a_flagbearer() {
        for (wave, has_flag) in [(4, false), (5, true), (60, false), (61, true), (62, true)] {
            assert_eq!(wave_plan(wave).flagbearer, has_flag, "wave {wave}");
        }
    }
}
