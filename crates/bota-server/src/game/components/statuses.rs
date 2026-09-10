//! What has been put on an entity and runs out on its own.

use bota_proto::DamageKind;

use crate::engine::Entity;
use crate::game::rules;

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
    /// Wears more armor and mends faster for standing by a tower of its own.
    Guarded {
        /// Armor added, in whole points.
        armor: i32,
        /// Health mended, in hundredths of a point a second.
        hp_per_second: i32,
    },
    /// Mends faster for marching beside the one carrying the flag.
    Inspired {
        /// Health mended, in hundredths of a point a second.
        hp_per_second: i32,
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
    /// Additional damage from subsequent Shadowrazes by the same caster.
    Shadowraze {
        /// The applying caster's full generational handle; server-only.
        from: Entity,
        /// Successful hits held, in `1..=rules::RAZE_MAX_STACKS`.
        stacks: u8,
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

    /// Replaces the same kind; Shadowraze replaces only the same caster's record.
    pub fn put(&mut self, status: Status) {
        let same = std::mem::discriminant(&status.kind);
        self.0.retain(|held| match (held.kind, status.kind) {
            (
                StatusKind::Shadowraze {
                    from: held_from, ..
                },
                StatusKind::Shadowraze {
                    from: next_from, ..
                },
            ) => held.ticks_left > 0 && held_from != next_from,
            _ => std::mem::discriminant(&held.kind) != same,
        });
        if let StatusKind::Shadowraze { stacks, .. } = status.kind {
            assert!(stacks > 0);
            assert!(status.ticks_left > 0);
            assert!(status.ticks_left <= rules::RAZE_DEBUFF_TICKS);
            let count = self
                .0
                .iter()
                .filter(|held| matches!(held.kind, StatusKind::Shadowraze { .. }))
                .count();
            assert!(count <= rules::RAZE_MAX_SOURCES);
            if count == rules::RAZE_MAX_SOURCES {
                let at = self
                    .0
                    .iter()
                    .enumerate()
                    .filter(|(_, held)| matches!(held.kind, StatusKind::Shadowraze { .. }))
                    .min_by_key(|(_, held)| held.ticks_left)
                    .map(|(at, _)| at)
                    .expect("a full Shadowraze source set has an expiry");
                self.0.remove(at);
            }
        }
        self.0.push(status);
    }

    /// Active Shadowraze hits held for this exact caster generation; zero when absent.
    pub fn raze_stacks(&self, caster: Entity) -> u8 {
        self.active()
            .find_map(|status| match status.kind {
                StatusKind::Shadowraze { from, stacks } if from == caster => Some(stacks),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Adds one successful hit and refreshes its caster's complete stack to 240 ticks.
    pub fn stack_raze(&mut self, caster: Entity) {
        let stacks = self.raze_stacks(caster).saturating_add(1);
        self.put(Status {
            kind: StatusKind::Shadowraze {
                from: caster,
                stacks,
            },
            ticks_left: rules::RAZE_DEBUFF_TICKS,
        });
        assert!(self.raze_stacks(caster) > 0);
        assert_eq!(self.raze_stacks(caster), stacks);
    }
}
