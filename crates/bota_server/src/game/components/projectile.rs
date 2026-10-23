use bota_proto::{DamageKind, Inflictor, ItemId};

use crate::game::{Entity, HitEffect};

/// A missile on its way to somebody.
///
/// It carries a [`Transform`] for where it is and a team of its own; whoever
/// threw it may be gone by the time it lands.
///
/// [`Transform`]: crate::game::Transform
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projectile {
    /// World units per second.
    pub speed: bota_proto::Fixed,
    /// Who threw it. The handle may outlive the body.
    pub source: Option<Entity>,
    /// The ability or item it carries. Absent for an attack's missile.
    pub inflictor: Option<Inflictor>,
    pub target: Entity,
    pub damage: i32,
    pub kind: DamageKind,
    /// Outgoing amplification of `damage`, captured when it was thrown.
    pub damage_amp_bp: i32,
    /// Elevation under its source when it was thrown.
    pub launch_tier: u8,
    /// Whether higher ground may evade it.
    pub can_miss_uphill: bool,
    /// Whether it goes through evasion and an uphill miss.
    pub pierces: bool,
    /// The magical damage each pierce that landed adds, and the item that
    /// pierced. Empty for none.
    pub pierced: Vec<(i32, ItemId)>,
    /// Outgoing amplification of what pierced, captured when thrown.
    pub pierce_amp_bp: i32,
    /// What its blow does besides its damage.
    pub effect: HitEffect,
}
