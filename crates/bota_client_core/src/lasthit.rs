//! How hard one's next plain attack lands on a unit, for marking what it
//! would kill.

use bota_proto::{Fixed, UnitKind, UnitView};

/// Armor points weigh this many hundredths of a hundred in the server's
/// armor rule.
const ARMOR_SCALE: i64 = 6;
/// The id of the Quelling Blade.
const QUELLING_BLADE: u16 = 5;
/// Attack damage a Quelling Blade in the inventory adds against creeps.
const QUELLING_BONUS: i32 = 18;
/// Inventory slots whose items work, of the nine carried.
const WORKING_SLOTS: usize = 6;

/// Physical damage left of `amount` after `armor`: `1 / (1 + 0.06 a)` of it
/// for positive armor, `2 - 1 / (1 + 0.06 |a|)` for negative, rounded down
/// as the server rounds.
pub fn after_armor(amount: i32, armor: Fixed) -> i32 {
    let whole = i64::from(Fixed::ONE.raw);
    let scaled = ARMOR_SCALE * i64::from(armor.raw).abs();
    let den = 100 * whole + scaled;
    let num = if armor < Fixed::ZERO {
        den + scaled
    } else {
        100 * whole
    };
    (i64::from(amount) * num / den).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Health one plain, uncritical attack of `attacker` takes off `target` as
/// both stand now.
pub fn next_hit(attacker: &UnitView, target: &UnitView) -> i32 {
    let quelling = attacker
        .items
        .iter()
        .take(WORKING_SLOTS)
        .flatten()
        .any(|item| item.id.0 == QUELLING_BLADE);
    let bonus = if quelling && is_creep(target.kind) {
        QUELLING_BONUS
    } else {
        0
    };
    after_armor(attacker.attack_damage + bonus, target.armor)
}

/// Whether a kind of unit is a lane or neutral creep.
pub fn is_creep(kind: UnitKind) -> bool {
    matches!(
        kind,
        UnitKind::CreepMelee
            | UnitKind::CreepFlagbearer
            | UnitKind::CreepRanged
            | UnitKind::CreepSiege
            | UnitKind::CreepNeutral
    )
}
