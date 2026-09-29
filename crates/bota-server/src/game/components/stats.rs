//! The numbers an entity fights by, worked out afresh every tick.

use bota_proto::{Attribute, Attributes, DamageKind, Fixed};

use crate::game::Ratio;

/// Everything the type an entity is, its level, its items and what is on it
/// add up to.
///
/// Written by the system that works stats out and read by everything else.
/// Nothing else writes here: a value put in by hand is gone next tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    /// The three attributes, after everything that adds to them.
    pub attributes: Attributes,
    /// Which attribute pays its attack damage. Absent for whatever has none.
    pub primary: Option<Attribute>,
    /// The most health it can hold.
    pub max_hp: Fixed,
    /// The most mana it can hold.
    pub max_mana: Fixed,
    /// Health mended each tick.
    pub hp_regen: Fixed,
    /// Mana mended each tick.
    pub mana_regen: Fixed,
    /// Damage one attack deals.
    pub damage: i32,
    /// Attack damage added against a creep and nothing else.
    pub damage_to_creeps: i32,
    /// How far it reaches, edge to edge.
    pub attack_range: Fixed,
    /// How far it looks for something to attack.
    pub acquisition: Fixed,
    /// Milliseconds between the starts of two attacks at
    /// [`rules::BASE_ATTACK_SPEED`].
    ///
    /// [`rules::BASE_ATTACK_SPEED`]: crate::game::rules::BASE_ATTACK_SPEED
    pub attack_time: u32,
    /// How fast it swings, where [`rules::BASE_ATTACK_SPEED`] is its own pace
    /// and twice that is twice the pace. Clamped to
    /// `MIN_ATTACK_SPEED..=MAX_ATTACK_SPEED` where read.
    ///
    /// [`rules::BASE_ATTACK_SPEED`]: crate::game::rules::BASE_ATTACK_SPEED
    pub attack_speed: i32,
    /// Milliseconds from the start of an attack to the hit at
    /// [`rules::BASE_ATTACK_SPEED`].
    ///
    /// [`rules::BASE_ATTACK_SPEED`]: crate::game::rules::BASE_ATTACK_SPEED
    pub attack_point: u32,
    /// Milliseconds after the hit before it may move again at
    /// [`rules::BASE_ATTACK_SPEED`].
    ///
    /// [`rules::BASE_ATTACK_SPEED`]: crate::game::rules::BASE_ATTACK_SPEED
    pub attack_backswing: u32,
    /// Speed of the missile it throws. Absent for a melee attack.
    pub projectile_speed: Option<Fixed>,
    /// Armor, reducing physical damage.
    pub armor: Fixed,
    /// Magic resistance, percent.
    pub magic_resist_pct: i32,
    /// Share taken off the durations of stuns, fears and slows, in basis
    /// points where 10_000 is everything and zero is nothing.
    pub status_resist_bp: i32,
    /// Scale on physical damage it deals, in basis points where 10_000 is
    /// nominal.
    pub physical_amp_bp: i32,
    /// Scale on magical damage it deals, in basis points where 10_000 is
    /// nominal.
    pub magic_amp_bp: i32,
    /// Scale on pure damage it deals, in basis points where 10_000 is
    /// nominal.
    pub pure_amp_bp: i32,
    /// Scale on every cooldown set on it, in basis points where 10_000 is
    /// nominal.
    pub cooldown_rate_bp: i32,
    /// Scale on every mana cost it pays, in basis points where 10_000 is
    /// nominal.
    pub mana_cost_rate_bp: i32,
    /// Share of attacks at it that miss.
    pub evasion: Ratio,
    /// Share of its attacks that pierce: go through evasion and an uphill
    /// miss, and land bonus magical damage.
    pub pierce: Ratio,
    /// Magical damage a pierce lands alongside the attack.
    pub pierce_damage: i32,
    /// World units per second on the ground.
    pub move_speed: Fixed,
    /// Brads per tick it turns.
    pub turn_rate: u16,
    /// How far it sees.
    pub vision: Fixed,
    /// How far it reveals what hides. Zero for whatever gives no true sight.
    pub true_sight: Fixed,
    /// Whether the other side sees it only through true sight.
    pub hides: bool,
    /// Whether it flies: closed ground is nothing to it.
    pub flies: bool,
    /// Whether it walks through the bodies in its way.
    pub phased: bool,
    /// Whether damage passes it by.
    pub invulnerable: bool,
}

impl Stats {
    /// Outgoing damage amplification of one kind, in basis points.
    pub fn damage_amp_bp(&self, kind: DamageKind) -> i32 {
        match kind {
            DamageKind::Physical => self.physical_amp_bp,
            DamageKind::Magical => self.magic_amp_bp,
            DamageKind::Pure => self.pure_amp_bp,
        }
    }
}
