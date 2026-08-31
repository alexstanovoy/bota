//! What has been put on an entity and runs out on its own.

use bota_proto::DamageKind;

use crate::engine::Entity;

/// One kind of effect, with what there is of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    /// Attacks come faster.
    Haste {
        /// Attack speed added.
        speed: i32,
    },
    /// Health mends faster.
    Mending {
        /// Hundredths of a point a tick.
        per_tick: i32,
        /// Whether a blow from a hero or a tower puts it out.
        breaks: bool,
    },
    /// Mana mends faster.
    Clarity {
        /// Hundredths of a point a tick.
        per_tick: i32,
        /// Whether a blow from a hero or a tower puts it out.
        breaks: bool,
    },
    /// Health and mana both mend faster, for standing in a fountain.
    Fountain {
        /// Hundredths of a point of health a tick.
        hp_per_tick: i32,
        /// Hundredths of a point of mana a tick.
        mana_per_tick: i32,
    },
    /// Cannot act at all: it neither walks, nor turns, nor swings, nor casts.
    Stunned,
    /// Nothing gets through: damage passes it by while it holds.
    Shielded,
    /// Walks through the bodies in its way, and nothing eases it out of one.
    Phased,
    /// Walks slower, by `pct` percent of what it would.
    Slowed {
        /// Percent taken off its speed.
        pct: i32,
    },
    /// Walks faster, by `pct` percent of what it would.
    Hastened {
        /// Percent added to its speed.
        pct: i32,
    },
    /// Wears less armor for standing near a dark presence.
    ArmorBroken {
        /// Armor taken off, in whole points.
        armor: i32,
    },
    /// Losing health over time to whoever put it on.
    Burning {
        /// Damage each beat of [`rules::BURN_PERIOD_TICKS`].
        ///
        /// [`rules::BURN_PERIOD_TICKS`]: crate::game::rules::BURN_PERIOD_TICKS
        amount: i32,
        /// Which reduction the damage answers to.
        kind: DamageKind,
        /// Who is dealing it, while that one still stands.
        from: Option<Entity>,
        /// Whether it may take the last point of health.
        lethal: bool,
    },
}

/// One effect on an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    /// What it does, and how much of it there is.
    pub kind: StatusKind,
    /// Ticks before it lifts.
    pub ticks_left: u32,
}

/// Everything on an entity right now. Absent when nothing is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Statuses(pub Vec<Status>);

impl Statuses {
    /// Every effect that has not run out.
    pub fn active(&self) -> impl Iterator<Item = &Status> {
        self.0.iter().filter(|s| s.ticks_left > 0)
    }

    /// Puts one on, in place of whatever of the same kind was already there.
    pub fn put(&mut self, status: Status) {
        let same = std::mem::discriminant(&status.kind);
        self.0
            .retain(|held| std::mem::discriminant(&held.kind) != same);
        self.0.push(status);
    }
}
