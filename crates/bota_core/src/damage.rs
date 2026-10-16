//! How a blow turns into health lost: the outgoing amplification, armor and
//! magic resistance, and how much a pool can give.

use bota_proto::{DamageKind, Fixed};

use crate::rules;

/// Damage after the outgoing amplification captured when the blow was made.
#[inline]
pub fn amplify(amount: i32, bp: i32) -> i32 {
    if bp == rules::NOMINAL_BP {
        return amount;
    }
    (i64::from(amount) * i64::from(bp) / i64::from(rules::NOMINAL_BP))
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Health `taken` comes off `hp` as: at most one past what is left, or down to
/// 1 hp and no further when `spared`.
#[inline]
pub fn bite(hp: Fixed, taken: i32, spared: bool) -> i32 {
    if spared {
        taken.min((hp - Fixed::from_int(1)).to_int().max(0))
    } else {
        taken.min(hp.to_int().max(0) + 1)
    }
}

/// The basis points of resistance a shown [`Fixed`] fraction stands for: the
/// inverse of `Fixed::from_ratio(bp, NOMINAL_BP)`, which a unit's view applies.
#[inline]
pub const fn magic_resist_bp(shown: Fixed) -> i32 {
    let raw = shown.raw as i64;
    let scale = 1i64 << Fixed::FRAC_BITS;
    let nominal = rules::NOMINAL_BP as i64;
    let magnitude = (raw.abs() * nominal + scale - 1) / scale;
    (if raw < 0 { -magnitude } else { magnitude }) as i32
}

/// Damage after armor or magic resistance.
#[inline]
pub fn mitigate(amount: i32, kind: DamageKind, armor: Fixed, magic_resist_bp: i32) -> i32 {
    match kind {
        DamageKind::Physical => {
            // Dota's curve: `1 / (1 + 0.06 a)` above zero, `2 - 1 / (1 + 0.06 |a|)`
            // below it.
            let whole = i64::from(Fixed::ONE.raw);
            let scaled = i64::from(rules::ARMOR_SCALE) * i64::from(armor.raw).abs();
            let den = 100 * whole + scaled;
            let num = if armor < Fixed::ZERO {
                den + scaled
            } else {
                100 * whole
            };
            (i64::from(amount) * num / den).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        }
        DamageKind::Magical => {
            let let_through = (rules::NOMINAL_BP - magic_resist_bp)
                .clamp(0, rules::NOMINAL_BP - rules::MIN_MAGIC_RESIST_BP);
            (i64::from(amount) * i64::from(let_through) / i64::from(rules::NOMINAL_BP))
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        }
        DamageKind::Pure => amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shown_resistance_gives_back_its_basis_points() {
        for bp in -rules::NOMINAL_BP..=rules::NOMINAL_BP {
            let shown = Fixed::from_ratio(bp, rules::NOMINAL_BP);
            assert_eq!(magic_resist_bp(shown), bp);
        }
    }
}
