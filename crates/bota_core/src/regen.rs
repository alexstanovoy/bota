//! Health and mana regeneration, one rule for the server that applies it and
//! for a client that shows it.

use bota_proto::{Attributes, Fixed, Team, UnitKind};

use crate::{ModifierKind, kind_def, rules};

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
#[inline]
fn per_second(hundredths: i32) -> Fixed {
    Fixed::from_ratio(hundredths, 100 * rules::TICKS_PER_SECOND as i32)
}

/// What one active modifier adds to regeneration.
#[inline]
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

/// Whether damage a unit of kind `source` on side `source_side` deals to a
/// unit on side `target_side` breaks what breaks on damage: a running mend
/// whose [`Mend::breaks`](crate::Mend::breaks) is set, and the wait of an item
/// with a [`breaks_on_damage`](crate::ItemDef::breaks_on_damage).
///
/// Dota 7.41f words it the same for each of them: Healing Salve and Clarity,
/// "If the unit is attacked by an enemy hero or Roshan, the effect is lost."
/// (the salve's `break_on_hero_damage`: any damage, attack or spell); Blink
/// Dagger, "cannot be used ... after taking damage from an enemy hero or
/// Roshan." A tower, a creep, a ward and one's own side break nothing.
#[inline]
pub fn breaks_regen(source: UnitKind, source_side: Team, target_side: Team) -> bool {
    source_side != target_side && kind_def(source).breaks_regen
}
