//! A blow on its way to being felt.

use bota_proto::DamageKind;

use crate::game::Entity;

/// Damage that has been dealt and not yet taken off anybody, queued on
/// [`World::hits`](crate::game::World::hits) and felt within the tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Who dealt it. The handle may outlive the body.
    pub source: Option<Entity>,
    /// Who takes it.
    pub target: Entity,
    /// Before armor and resistance.
    pub amount: i32,
    /// Which reduction applies.
    pub kind: DamageKind,
    /// Outgoing amplification captured when the blow was made, in basis
    /// points where 10_000 is nominal.
    pub damage_amp_bp: i32,
    /// Whether it was a critical strike.
    pub crit: bool,
    /// Whether it is a swing of the attacker's weapon. Only such a blow can
    /// be evaded.
    pub attack: bool,
    /// Whether it goes through evasion.
    pub pierces: bool,
    /// What else the blow does.
    pub effect: HitEffect,
}

/// What a blow does besides its damage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitEffect {
    /// Nothing.
    None,
    /// Adds [`rules::RAZE_STACK_DAMAGE`] per Shadowraze stack the source
    /// already holds on the target, and stacks one more on a target that
    /// takes damage and survives. `level` is the ability level, from zero.
    ///
    /// [`rules::RAZE_STACK_DAMAGE`]: crate::game::rules::RAZE_STACK_DAMAGE
    Shadowraze { level: u8 },
}
