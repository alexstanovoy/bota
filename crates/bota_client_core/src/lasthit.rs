//! How hard one's next plain attack lands on a unit, for marking what it
//! would kill.

use bota_core::{
    Carried, CarriedItem, attack_class_pct, carried_bonus, hero_def, kind_def, magic_resist_bp,
    mitigate, rules,
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

/// What the items a unit's working inventory carries add: a muted stack and
/// the backpack are inert.
pub fn carried_of(unit: &UnitView) -> Carried {
    carried_bonus(
        unit.items
            .iter()
            .take(rules::INVENTORY_SLOTS)
            .flatten()
            .filter(|item| item.mute_left == 0)
            .map(|item| CarriedItem {
                id: item.id,
                charges: item.charges.unwrap_or(0),
                mode: item.mode,
            }),
        unit.hero
            .and_then(hero_def)
            .is_some_and(|hero| hero.unit.projectile_speed.is_some()),
    )
}

/// `amount` of magical damage after the magic resistance `target` shows.
pub fn magic_taken(amount: i32, target: &UnitView) -> i32 {
    mitigate(
        amount,
        DamageKind::Magical,
        Fixed::ZERO,
        magic_resist_bp(target.magic_resist),
    )
}

/// Health one plain, uncritical attack of `attacker` takes off `target` by
/// the server's own rules, when the target wears `armor` as the blow lands and
/// the attacker's base damage is `raise` above what its view shows (damage
/// gained before the blow, such as souls from kills on the way). A raise of
/// `roll` makes `least` the swing that draws `roll` above the range's floor.
///
/// Item bonuses count as the working inventory carries them (a muted stack
/// and the backpack are inert). The outgoing amplification a spawn modifier
/// may add is hidden and read as nominal; whether a pierce
/// lands is a roll, so it only raises `most`.
pub fn next_hit_raised(
    attacker: &UnitView,
    target: &UnitView,
    armor: Fixed,
    raise: i32,
) -> HitRange {
    let carried = carried_of(attacker);
    let against = kind_def(target.kind);
    let creeps = if against.creep {
        carried.damage_to_creeps
    } else {
        0
    };
    let class_pct = attack_class_pct(attacker.kind, target.kind);
    let swing = |damage: i32| {
        // Saturating: the attacker's numbers are a peer's word.
        let blow = damage.saturating_add(creeps).saturating_mul(class_pct) / 100;
        mitigate(blow, DamageKind::Physical, armor, 0)
    };
    let least = swing(attacker.attack_damage_min.saturating_add(raise));
    let mid = swing(attacker.attack_damage.max(0).saturating_add(raise));
    let mut most = swing(attacker.attack_damage_max.saturating_add(raise));
    if !against.structure && carried.pierce_damage > 0 {
        let pierce = carried.pierce_damage.saturating_mul(class_pct) / 100;
        most = most.saturating_add(magic_taken(pierce, target));
    }
    HitRange { least, mid, most }
}
