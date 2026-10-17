//! What a modifier does to whatever wears it, and how much of it there is.

use bota_proto::{DamageKind, EffectId, Fixed};

use crate::rules;

/// One kind of modifier, with what there is of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierKind {
    /// Health mends faster.
    Mending {
        /// Hundredths of a point a tick.
        per_tick: i32,
        /// Whether a blow that [`breaks_regen`](crate::breaks_regen) puts it out.
        breaks: bool,
    },
    /// Mana mends faster.
    Clarity {
        /// Hundredths of a point a tick.
        per_tick: i32,
        /// Whether a blow that [`breaks_regen`](crate::breaks_regen) puts it out.
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
        /// Armor taken off.
        armor: Fixed,
    },
    /// Keeps less magic out for having been crossed by a requiem: what its
    /// resistance lets through grows by `pct` percent.
    MagicBroken {
        /// Percent added to the share of magic damage let through.
        pct: i32,
    },
    /// Wears more armor and mends faster for standing by a tower of its own.
    Guarded {
        /// Armor added, in whole points.
        armor: i32,
        /// Health mended, in hundredths of a point a second.
        hp_per_second: i32,
    },
    /// Mends faster and shrugs off more magic for marching beside the one
    /// carrying the flag.
    Inspired {
        /// Health mended, in hundredths of a point a second.
        hp_per_second: i32,
        /// Magic resistance, percent, multiplying with what is already there.
        magic_resist_pct: i32,
    },
    /// Losing health over time to whoever put it on.
    Burning {
        /// Damage each beat of [`rules::BURN_PERIOD_TICKS`].
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
    /// Walks slower by [`rules::GRENADE_SLOW_PCT`] percent, and takes
    /// [`rules::GRENADE_DAMAGE_PER_SECOND`] magical damage from whoever put it
    /// on every whole second after `since`.
    BloodGrenade {
        /// The tick the first Blood Grenade of the run landed.
        since: u32,
    },
}

impl ModifierKind {
    /// The number the wire names this kind of modifier with.
    pub const fn effect_id(self) -> EffectId {
        match self {
            ModifierKind::Mending { .. } => EffectId::MENDING,
            ModifierKind::Clarity { .. } => EffectId::CLARITY,
            ModifierKind::Fountain { .. } => EffectId::FOUNTAIN,
            ModifierKind::Stunned => EffectId::STUNNED,
            ModifierKind::Shielded => EffectId::SHIELDED,
            ModifierKind::Slowed { .. } => EffectId::SLOWED,
            ModifierKind::Hastened { .. } => EffectId::HASTENED,
            ModifierKind::Burning { .. } => EffectId::BURNING,
            ModifierKind::Phased => EffectId::PHASED,
            ModifierKind::ArmorBroken { .. } => EffectId::ARMOR_BROKEN,
            ModifierKind::MagicBroken { .. } => EffectId::MAGIC_BROKEN,
            ModifierKind::Guarded { .. } => EffectId::GUARDED,
            ModifierKind::Inspired { .. } => EffectId::INSPIRED,
            ModifierKind::Shadowraze { .. } => EffectId::SHADOWRAZE,
            ModifierKind::Rot { .. } => EffectId::ROT,
            ModifierKind::Feared => EffectId::FEARED,
            ModifierKind::BloodGrenade { .. } => EffectId::BLOOD_GRENADE,
        }
    }

    /// What an aura of this kind hands out from a source that spawned after
    /// `steps` upgrade intervals. Only the flagbearer's inspiration grows.
    #[inline]
    pub fn grown(self, steps: u32) -> ModifierKind {
        match self {
            ModifierKind::Inspired {
                hp_per_second,
                magic_resist_pct,
            } => {
                let steps = steps.min(rules::FLAGBEARER_AURA_MAGIC_RESIST_CAP) as i32;
                ModifierKind::Inspired {
                    hp_per_second,
                    magic_resist_pct: magic_resist_pct
                        + rules::FLAGBEARER_AURA_MAGIC_RESIST_PCT * steps,
                }
            }
            other => other,
        }
    }
}
