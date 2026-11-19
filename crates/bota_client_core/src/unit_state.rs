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

