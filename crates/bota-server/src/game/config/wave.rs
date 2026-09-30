//! What a creep wave is made of, and how it grows with the clock.

use bota_proto::Vec2;

use crate::game::{isqrt64, rules};

/// How strong one kind of a side's lane creeps is, by which enemy barracks
/// have fallen. Always [`Normal`](CreepRank::Normal) on a map with no
/// barracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreepRank {
    /// Neither of the others.
    Normal,
    /// The enemy barracks of the lane matching its kind destroyed: melee or
    /// ranged, both of them for siege.
    Super,
    /// Every enemy barracks destroyed.
    Mega,
}

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
    /// Upgrades the wave's creeps carry. Siege creeps and the flagbearer
    /// gain nothing from them.
    pub upgrades: u32,
}

impl WavePlan {
    /// Creeps in the wave, all kinds counted.
    pub fn size(&self) -> u32 {
        self.melee + self.ranged + self.siege
    }
}

/// The wave due at a tick, counting from one, if a wave is due at all.
pub fn wave_at(tick: u32) -> Option<u32> {
    if tick < rules::FIRST_WAVE_TICK
        || !(tick - rules::FIRST_WAVE_TICK).is_multiple_of(rules::WAVE_PERIOD_TICKS)
    {
        return None;
    }
    Some((tick - rules::FIRST_WAVE_TICK) / rules::WAVE_PERIOD_TICKS + 1)
}

/// Upgrades a wave's creeps carry: one per [`rules::WAVE_UPGRADE_PERIOD`]
/// waves, up to [`rules::WAVE_UPGRADE_CAP`].
pub fn wave_upgrades(wave: u32) -> u32 {
    ((wave - 1) / rules::WAVE_UPGRADE_PERIOD).min(rules::WAVE_UPGRADE_CAP)
}

/// What the given wave is made of.
pub fn wave_plan(wave: u32) -> WavePlan {
    let mut melee = rules::MELEE_PER_WAVE;
    for &(from, count) in &rules::MELEE_GROWTH {
        if wave >= from {
            melee = count;
        }
    }
    let mut ranged = rules::RANGED_PER_WAVE;
    if wave >= rules::RANGED_GROWTH_WAVE {
        ranged = rules::RANGED_PER_WAVE + 1;
    }
    let siege = if wave < rules::FIRST_SIEGE_WAVE
        || !(wave - rules::FIRST_SIEGE_WAVE).is_multiple_of(rules::SIEGE_WAVE_PERIOD)
    {
        0
    } else if wave >= rules::SIEGE_GROWTH_WAVE {
        2
    } else {
        1
    };
    let flagbearer = wave >= rules::FIRST_FLAGBEARER_WAVE
        && (wave - rules::FIRST_FLAGBEARER_WAVE).is_multiple_of(rules::FLAGBEARER_WAVE_PERIOD);
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
    let len = isqrt64(
        i64::from(forward.x.raw) * i64::from(forward.x.raw)
            + i64::from(forward.y.raw) * i64::from(forward.y.raw),
    );
    // A wave with nowhere to march lines up along the x axis. The length
    // divides last: divided first, the raw direction rounds to zero.
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
    let spread = |i: u32, of: u32| {
        let step = rules::WAVE_SPAWN_SPACING;
        (i as i32 - (of as i32 - 1) / 2) * step
    };
    for i in 0..front {
        out.push(across(spread(i, front)));
    }
    for i in 0..plan.ranged {
        out.push(across(spread(i, plan.ranged)) + along(-rules::WAVE_SPAWN_RANK));
    }
    out
}
