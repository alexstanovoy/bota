//! Health and mana regeneration, one rule for the server that applies it and
//! for a client that shows it.

use bota_proto::{Attributes, Fixed};

use crate::{ModifierKind, rules};

/// What a unit regenerates each tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Regen {
    /// Health per tick.
    pub hp: Fixed,
    /// Mana per tick.
    pub mana: Fixed,
}

impl Regen {
    /// Nothing regenerates.
    pub const NONE: Regen = Regen {
        hp: Fixed::ZERO,
        mana: Fixed::ZERO,
    };
}

/// Health or mana per tick from hundredths of a point per second.
fn per_second(hundredths: i32) -> Fixed {
    Fixed::from_ratio(hundredths, 100 * rules::TICKS_PER_SECOND as i32)
}

/// What one active modifier adds to regeneration.
fn from_modifier(kind: ModifierKind) -> Regen {
    let (hp, mana) = match kind {
        ModifierKind::Mending { per_tick, .. } => (Fixed::from_ratio(per_tick, 100), Fixed::ZERO),
        ModifierKind::Clarity { per_tick, .. } => (Fixed::ZERO, Fixed::from_ratio(per_tick, 100)),
        ModifierKind::Fountain {
            hp_per_tick,
            mana_per_tick,
        } => (
            Fixed::from_ratio(hp_per_tick, 100),
            Fixed::from_ratio(mana_per_tick, 100),
        ),
        ModifierKind::Guarded { hp_per_second, .. }
        | ModifierKind::Inspired { hp_per_second, .. } => (per_second(hp_per_second), Fixed::ZERO),
        _ => (Fixed::ZERO, Fixed::ZERO),
    };
    Regen { hp, mana }
}

/// Everything a unit regenerates: its kind's own base, what its items carry
/// (`carried`), its strength and intelligence at their final worth, and the
/// active modifiers on it (consumables, the fountain, auras).
pub fn regen(
    base: Regen,
    carried: Regen,
    attributes: Attributes,
    modifiers: impl Iterator<Item = ModifierKind>,
) -> Regen {
    let mut total = Regen {
        hp: base.hp + carried.hp + rules::HP_REGEN_PER_STRENGTH * attributes.strength,
        mana: base.mana
            + carried.mana
            + rules::MANA_REGEN_PER_INTELLIGENCE * attributes.intelligence,
    };
    for kind in modifiers {
        let added = from_modifier(kind);
        total.hp += added.hp;
        total.mana += added.mana;
    }
    total
}
