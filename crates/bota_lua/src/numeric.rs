//! Checked input domains for projections over received statistics.

use bota_proto::Fixed;

/// Core physical mitigation uses signed 64-bit products before division.
/// Keep its normal results and reject observations outside that intermediate domain.
pub(crate) fn physical_damage(amount: i32, armor: Fixed) -> Option<i32> {
    bota_client_core::observed_physical_damage(amount, armor)
}
