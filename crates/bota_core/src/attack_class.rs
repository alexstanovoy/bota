//! Dota's attack classes: how much of an attack's damage reaches a target,
//! by what struck and what was struck, before armor.

use bota_proto::UnitKind;

use crate::kind_def;

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

/// Percent of an attack's damage that reaches a target, by the attacker's
/// kind and the target's.
#[inline]
pub fn attack_class_pct(attacker: UnitKind, target: UnitKind) -> i32 {
    match (
        kind_def(attacker).attack_class,
        kind_def(target).attack_class,
    ) {
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
