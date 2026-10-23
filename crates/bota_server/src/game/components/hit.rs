//! A blow on its way to being felt.

use bota_proto::{DamageKind, Inflictor};

use crate::game::Entity;

/// Damage that has been dealt and not yet taken off anybody, queued on
/// [`World::hits`](crate::game::World::hits) and felt within the tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Who dealt it. The handle may outlive the body.
    pub source: Option<Entity>,
    /// The ability or item that dealt it, which also puts on what its
    /// effect leaves. Absent for an attack's own blow.
    pub inflictor: Option<Inflictor>,
    pub target: Entity,
    /// Before armor and resistance.
    pub amount: i32,
    pub kind: DamageKind,
    /// Outgoing amplification captured when the blow was made, in basis
    /// points where 10_000 is nominal.
    pub damage_amp_bp: i32,
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
    /// Adds `stack_damage` per Shadowraze stack the source already holds on
    /// the target, and stacks one more on a target that takes damage and
    /// survives. `stack_damage` is the cast level's
    /// [`rules::RAZE_STACK_DAMAGE`] and the caster's talent, as the raze went
    /// off.
    ///
    /// [`rules::RAZE_STACK_DAMAGE`]: bota_core::rules::RAZE_STACK_DAMAGE
    Shadowraze { stack_damage: i32 },
    /// Frightens, slows and tears the magic resistance of a target that
    /// takes damage and survives, for `fear.line` ticks more, up to
    /// `fear.most`. `level` is the requiem's, from zero.
    Requiem { level: u8, fear: Fear },
    /// Stuns a target that survives for `ticks`, whether or not the blow
    /// took any health: a neutral's bolt that does no damage still holds.
    Stun { ticks: u32 },
    /// Roots a target that survives for `ticks`, the same way.
    Root { ticks: u32 },
    /// Heals the source by the health the blow took, one bite of a dismember.
    Dismember,
}

impl HitEffect {
    /// Whether it lands on a target that survives a blow taking nothing off
    /// it.
    pub const fn holds(self) -> bool {
        matches!(self, HitEffect::Stun { .. } | HitEffect::Root { .. })
    }
}

/// How long a requiem line holds what it burns: ticks a line adds, and the
/// most it may hold in all. Set as the requiem is let go, talents included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fear {
    /// Ticks one line adds.
    pub line: u32,
    /// The most ticks the hold reaches.
    pub most: u32,
}
