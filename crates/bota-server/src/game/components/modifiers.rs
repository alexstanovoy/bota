//! What has been put on an entity: what it does, who put it, how long it
//! holds.

use bota_proto::DamageKind;

use crate::engine::Entity;
use crate::game::rules;

/// One kind of modifier, with what there is of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierKind {
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
        /// Whether it may take the last point of health.
        lethal: bool,
    },
    /// Additional damage from subsequent Shadowrazes by whoever put it on.
    Shadowraze {
        /// Successful hits held, in `1..=rules::RAZE_MAX_STACKS`.
        stacks: u8,
    },
    /// The rot, switched on.
    Rot {
        /// Which level of it is running, counted from zero.
        level: u8,
    },
    /// Runs from whoever put it on, and neither swings nor casts, until it
    /// lifts.
    Feared,
}

/// One modifier on an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modifier {
    /// What it does, and how much of it there is.
    pub kind: ModifierKind,
    /// Who put it on. Absent for the world's own.
    pub source: Option<Entity>,
    /// Ticks before it lifts. Absent for one that does not lift on its own.
    pub ticks_left: Option<u32>,
}

/// Everything on an entity right now. Present on every unit; empty when
/// nothing is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(pub Vec<Modifier>);

impl Modifiers {
    /// Every modifier that has not run out.
    pub fn active(&self) -> impl Iterator<Item = &Modifier> {
        self.0.iter().filter(|held| held.ticks_left != Some(0))
    }

    /// Replaces the same kind; Shadowraze replaces only the same source's
    /// record.
    pub fn put(&mut self, modifier: Modifier) {
        let same = std::mem::discriminant(&modifier.kind);
        self.0.retain(|held| match (held.kind, modifier.kind) {
            (ModifierKind::Shadowraze { .. }, ModifierKind::Shadowraze { .. }) => {
                held.ticks_left != Some(0) && held.source != modifier.source
            }
            _ => std::mem::discriminant(&held.kind) != same,
        });
        if let ModifierKind::Shadowraze { stacks } = modifier.kind {
            assert!(stacks > 0);
            let left = modifier.ticks_left.expect("a Shadowraze record runs out");
            assert!(left > 0);
            assert!(left <= rules::RAZE_DEBUFF_TICKS);
            let count = self
                .0
                .iter()
                .filter(|held| matches!(held.kind, ModifierKind::Shadowraze { .. }))
                .count();
            assert!(count <= rules::RAZE_MAX_SOURCES);
            if count == rules::RAZE_MAX_SOURCES {
                let at = self
                    .0
                    .iter()
                    .enumerate()
                    .filter(|(_, held)| matches!(held.kind, ModifierKind::Shadowraze { .. }))
                    .min_by_key(|(_, held)| held.ticks_left.unwrap_or(u32::MAX))
                    .map(|(at, _)| at)
                    .expect("a full Shadowraze source set has an expiry");
                self.0.remove(at);
            }
        }
        self.0.push(modifier);
    }

    /// Takes off every modifier of one kind from one source, whatever there
    /// is of it.
    pub fn take(&mut self, like: ModifierKind, source: Option<Entity>) {
        let same = std::mem::discriminant(&like);
        self.0
            .retain(|held| std::mem::discriminant(&held.kind) != same || held.source != source);
    }

    /// Active Shadowraze hits held for this exact caster generation; zero
    /// when absent.
    pub fn raze_stacks(&self, caster: Entity) -> u8 {
        self.active()
            .find_map(|held| match held.kind {
                ModifierKind::Shadowraze { stacks } if held.source == Some(caster) => Some(stacks),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Adds one successful hit and refreshes its caster's complete stack to
    /// [`rules::RAZE_DEBUFF_TICKS`].
    pub fn stack_raze(&mut self, caster: Entity) {
        let stacks = self.raze_stacks(caster).saturating_add(1);
        self.put(Modifier {
            kind: ModifierKind::Shadowraze { stacks },
            source: Some(caster),
            ticks_left: Some(rules::RAZE_DEBUFF_TICKS),
        });
        assert!(self.raze_stacks(caster) > 0);
        assert_eq!(self.raze_stacks(caster), stacks);
    }
}
