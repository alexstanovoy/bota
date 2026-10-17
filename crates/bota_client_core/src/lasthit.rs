//! How hard one's next plain attack lands on a unit, for marking what it
//! would kill.

use bota_core::{
    CarriedItem, attack_class_pct, carried_bonus, hero_def, kind_def, magic_resist_bp, mitigate,
    rules,
};
use bota_proto::{DamageKind, Fixed, UnitView};

/// The health one plain attack takes off a target: the weakest swing, the
/// shown middle of the damage range, and the strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitRange {
    /// The weakest swing, with no pierce riding on it.
    pub least: i32,
    /// The swing of the shown attack damage, the middle of the range.
    pub mid: i32,
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
    next_hit_against(attacker, target, target.armor)
}

/// [`next_hit`] on a target that wears `armor` when the blow lands.
pub fn next_hit_against(attacker: &UnitView, target: &UnitView, armor: Fixed) -> HitRange {
    next_hit_raised(attacker, target, armor, 0)
}

/// [`next_hit_against`] for a swing whose base damage is `raise` above what the
/// attacker's view shows when it lands: damage the attacker gains before the
/// blow, such as souls from kills on the way. A raise of `roll` makes `least` the
/// swing that draws `roll` above the range's floor.
pub fn next_hit_raised(
    attacker: &UnitView,
    target: &UnitView,
    armor: Fixed,
    raise: i32,
) -> HitRange {
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
        attacker
            .hero
            .and_then(hero_def)
            .is_some_and(|hero| hero.unit.projectile_speed.is_some()),
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
        mitigate(blow, DamageKind::Physical, armor, 0)
    };
    let least = swing(attacker.attack_damage_min + raise);
    let mid = swing(attacker.attack_damage.max(0) + raise);
    let mut most = swing(attacker.attack_damage_max + raise);
    if !against.structure && carried.pierce_damage > 0 {
        let resist = magic_resist_bp(target.magic_resist);
        most += mitigate(
            carried.pierce_damage * class_pct / 100,
            DamageKind::Magical,
            Fixed::ZERO,
            resist,
        );
    }
    HitRange { least, mid, most }
}
