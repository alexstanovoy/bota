//! Pure catalogue timing queries, preserving imported milliseconds where available.
use crate::{Ms, rules};
use bota_proto::{AbilityId, ItemId};
/// O(imported ability rows), a fixed table. Imported channels retain sub-tick precision;
/// built-in utility abilities use their tick-defined rules. `level` is one-based, zero uses rank one.
pub fn ability_channel_time(id: AbilityId, level: u8) -> Option<Ms> {
    let definition = crate::ability_def(id)?;
    if let Some(data) = crate::ability_data(definition.key) {
        let rank = usize::from(level.saturating_sub(1));
        return data
            .channel
            .get(rank)
            .or_else(|| data.channel.last())
            .copied();
    }
    Some(Ms(
        (u64::from(definition.duration) * 1000 / u64::from(rules::TICKS_PER_SECOND))
            .min(u64::from(u32::MAX)) as u32,
    ))
}
/// O(1). Item channels in this catalogue are whole seconds; this does not infer remaining time.
pub fn item_channel_time(id: ItemId) -> Option<Ms> {
    crate::item_def(id).map(|definition| {
        Ms(
            (u64::from(definition.duration) * 1000 / u64::from(rules::TICKS_PER_SECOND))
                .min(u64::from(u32::MAX)) as u32,
        )
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imported_channel_milliseconds_are_not_rounded_through_simulation_ticks() {
        assert_eq!(
            ability_channel_time(AbilityId::DISMEMBER, 1),
            Some(Ms(2750))
        );
        assert_eq!(ability_channel_time(AbilityId::RAZE_NEAR, 1), Some(Ms(0)));
    }
    #[test]
    fn every_current_item_channel_retains_exact_whole_second_precision() {
        for item in crate::ITEMS {
            assert!(
                item.duration.is_multiple_of(rules::TICKS_PER_SECOND),
                "{} needs imported millisecond channel metadata",
                item.key
            );
        }
    }
}
