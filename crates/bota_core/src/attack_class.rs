//! Dota's attack classes: how much of an attack's damage reaches a target,
//! by what struck and what was struck, before armor.

use bota_proto::UnitKind;

use crate::is_structure;

/// How a kind of unit's attacks and body answer the class table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackClass {
    /// A hero: full damage but to what is reinforced.
    Hero,
    /// Melee and flagbearer lane creeps.
    Runty,
    /// Ranged lane creeps.
    Piercing,
    /// Siege creeps and every building.
    Reinforced,
    /// Everything else, the jungle included: full damage everywhere.
    Plain,
}

/// The class a kind of unit belongs to.
#[inline]
pub fn attack_class_of(kind: UnitKind) -> AttackClass {
    match kind {
        UnitKind::Hero => AttackClass::Hero,
        UnitKind::CreepMelee | UnitKind::CreepFlagbearer => AttackClass::Runty,
        UnitKind::CreepRanged => AttackClass::Piercing,
        UnitKind::CreepSiege => AttackClass::Reinforced,
        kind if is_structure(kind) => AttackClass::Reinforced,
        _ => AttackClass::Plain,
    }
}

/// Percent of an attack's damage that reaches a target, by the attacker's
/// kind and the target's.
#[inline]
pub fn attack_class_pct(attacker: UnitKind, target: UnitKind) -> i32 {
    let target = attack_class_of(target);
    match (attack_class_of(attacker), target) {
        (AttackClass::Runty, AttackClass::Hero) => 75,
        (AttackClass::Runty, AttackClass::Reinforced) => 70,
        (AttackClass::Piercing, AttackClass::Hero) => 50,
        (AttackClass::Piercing, AttackClass::Reinforced) => 35,
        (AttackClass::Piercing, _) => 150,
        (AttackClass::Hero, AttackClass::Reinforced) => 50,
        (AttackClass::Reinforced, AttackClass::Reinforced) => 250,
        _ => 100,
    }
}
