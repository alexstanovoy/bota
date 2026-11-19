//! Control facts of a received unit, set by the simulation's authoritative predicates.
use bota_proto::{StatusFlags, UnitView};
/// O(1), without reconstructing modifier rules from display effects.
pub fn is_silenced(unit: &UnitView) -> bool {
    unit.statuses.bits & StatusFlags::SILENCED != 0
}
/// O(1), the received root predicate.
pub fn is_rooted(unit: &UnitView) -> bool {
    unit.statuses.bits & StatusFlags::ROOTED != 0
}
/// O(1), the received disarm predicate, including ethereal units.
pub fn is_disarmed(unit: &UnitView) -> bool {
    unit.statuses.bits & StatusFlags::DISARMED != 0
}

/// O(1). An absent received modifier index has the reference sentinel -1.
pub fn modifier_stack_count(unit: &UnitView, index: i32) -> i64 {
    usize::try_from(index)
        .ok()
        .and_then(|index| unit.effects.get(index))
        .map_or(-1, |effect| i64::from(effect.stacks.unwrap_or(0)))
}
