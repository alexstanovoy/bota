//! A neutral boss's homes and lifecycle, independent of a particular map.
use crate::NeutralKind;
use bota_proto::{ObjectiveKind, Vec2};

#[derive(Clone, Copy, Debug)]
pub struct LairDef {
    pub objective: ObjectiveKind,
    pub neutral: NeutralKind,
    pub day: Vec2,
    pub night: Vec2,
    /// Seconds after the horn before the natural day/night cycle chooses the home.
    pub move_after: u32,
    /// Delay after a death, inclusive, in seconds.
    pub respawn: [u32; 2],
    pub drops: &'static [LairDrop],
    /// The intrinsic ability describing this boss's journey between homes.
    pub moving: Option<crate::NeutralAbility>,
}

#[derive(Clone, Copy, Debug)]
pub struct LairDrop {
    pub item: bota_proto::ItemId,
    pub from: u32,
    pub reclaimed: bool,
    /// Drop levels advance with each qualifying fall, capped here.
    pub max_level: u8,
}

/// A boss's movement between homes, including the casts that interrupt it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Migration {
    pub effect: bota_proto::EffectId,
    pub interrupts: &'static [crate::NeutralAbility],
    pub max_stops: u8,
    pub speed_per_stop: i32,
    pub max_speed_bonus: i32,
    pub slow_resist_pct: i32,
    pub contact: Contact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contact {
    pub radius: i32,
    pub damage: i32,
    pub distance: i32,
    pub duration: u32,
    pub height: i32,
    pub cooldown: u32,
    pub tree_radius: i32,
    pub effect: bota_proto::EffectId,
    pub timeout_effect: bota_proto::EffectId,
}

impl LairDef {
    /// The publicly scheduled home; artificial sky changes do not change it.
    pub fn home_at(&self, clock: u32, schedule: crate::rules::Schedule) -> Vec2 {
        let starts = schedule.horn.saturating_add(
            self.move_after
                .saturating_mul(crate::rules::TICKS_PER_SECOND),
        );
        if clock >= starts && crate::time_of_day(clock, schedule).night {
            self.night
        } else {
            self.day
        }
    }
}
