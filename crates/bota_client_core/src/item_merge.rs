//! Item-wait projection through the shared charge-merge contract.

use bota_core::{Counts, HeldItem, merge_counts, merge_room};
use bota_proto::{DispenserView, ItemId, ItemView};

/// Applies the server's shared charge-merge contract to two item views.
/// The source may be empty afterwards; false leaves both views unchanged.
pub(crate) fn merge_item_views(
    target: &mut ItemView,
    source: &mut ItemView,
    combine: bool,
) -> bool {
    let room = merge_room(target, source, combine);
    if room == 0 {
        return false;
    }
    let counts = |item: &ItemView| Counts {
        id: item.id,
        charges: HeldItem::charges(item),
        sentries: item.sentries(),
    };
    let (merged, left) = merge_counts(counts(target), counts(source), room);
    let active = if target.id == ItemId::WARD_DISPENSER {
        target.dispenser.is_some_and(|d| d.sentry_active)
    } else {
        source.dispenser.is_some_and(|d| d.sentry_active)
    };
    set_counts(target, merged, active);
    let source_active = source.dispenser.is_some_and(|d| d.sentry_active);
    set_counts(source, left, source_active);
    target.cooldown_left = target.cooldown_left.max(source.cooldown_left);
    target.mute_left = target.mute_left.max(source.mute_left);
    target.locked = Some(target.locked == Some(true) || source.locked == Some(true));
    true
}

fn set_counts(item: &mut ItemView, counts: Counts, sentry_active: bool) {
    item.id = counts.id;
    item.dispenser = (counts.id == ItemId::WARD_DISPENSER).then_some(DispenserView {
        observers: counts.charges,
        sentries: counts.sentries,
        sentry_active,
    });
    item.charges = Some(if item.dispenser.is_some_and(|d| d.sentry_active) {
        counts.sentries
    } else {
        counts.charges
    });
}
