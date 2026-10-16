//! How hard one's next plain attack lands on a unit, for marking what it
//! would kill.

use bota_core::{CarriedItem, attack_class_pct, carried_bonus, kind_def, mitigate, rules};
use bota_proto::{DamageKind, Fixed, UnitView};

/// The health one plain attack takes off a target, between the weakest swing
/// and the strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitRange {
    /// The weakest swing, with no pierce riding on it.
    pub least: i32,
    /// The strongest swing, with the pierce it may carry.
    pub most: i32,
}

/// Health one plain, uncritical attack of `attacker` takes off `target` as
/// both stand now, by the server's own rules.
///
/// Item bonuses count as the working inventory carries them (a muted stack
/// and the backpack are inert). The outgoing amplification a
/// spawn modifier may add is hidden and read as nominal; whether a pierce
/// lands is a roll, so it only raises `most`.
pub fn next_hit(attacker: &UnitView, target: &UnitView) -> HitRange {
    let carried = carried_bonus(
        attacker
            .items
            .iter()
            .take(rules::INVENTORY_SLOTS)
            .flatten()
            .filter(|item| item.mute_left == 0)
            .map(|item| CarriedItem {
                id: item.id,
                charges: item.charges.unwrap_or(0),
                mode: item.mode,
            }),
    );
    let against = kind_def(target.kind);
    let creeps = if against.creep {
        carried.damage_to_creeps
    } else {
        0
    };
    let class_pct = attack_class_pct(attacker.kind, target.kind);
    let swing = |damage: i32| {
        let blow = (damage + creeps) * class_pct / 100;
        mitigate(blow, DamageKind::Physical, target.armor, 0)
    };
    let least = swing(attacker.attack_damage_min);
    let mut most = swing(attacker.attack_damage_max);
    if !against.structure && carried.pierce_damage > 0 {
        let resist = magic_resist_bp(target.magic_resist);
        most += mitigate(
            carried.pierce_damage * class_pct / 100,
            DamageKind::Magical,
            Fixed::ZERO,
            resist,
        );
    }
    HitRange { least, most }
}

/// A magic resistance fraction in basis points, as the server holds it.
fn magic_resist_bp(resist: Fixed) -> i32 {
    ((i64::from(resist.raw) * i64::from(rules::NOMINAL_BP)) >> 16) as i32
}
