//! Checked input domains for projections over received statistics.

use bota_proto::{DamageKind, Fixed};

/// Core physical mitigation uses signed 64-bit products before division.
/// Keep its normal results and reject observations outside that intermediate domain.
pub(crate) fn physical_damage(amount: i32, armor: Fixed) -> Option<i32> {
    let factor = 100 * i128::from(Fixed::ONE.raw)
        + 2 * i128::from(bota_core::rules::ARMOR_SCALE) * i128::from(armor.raw).abs();
    let product = i128::from(amount) * factor;
    if armor < Fixed::ZERO && !(i128::from(i64::MIN)..=i128::from(i64::MAX)).contains(&product) {
        return None;
    }
    Some(bota_core::mitigate(amount, DamageKind::Physical, armor, 0))
}
