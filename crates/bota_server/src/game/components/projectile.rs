use bota_proto::DamageKind;

use crate::game::Entity;

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
    /// Magical damage it lands alongside its own, from a pierce. Zero for
    /// none.
    pub pierce_damage: i32,
    /// Outgoing amplification of `pierce_damage`, captured when thrown.
    pub pierce_amp_bp: i32,
}
