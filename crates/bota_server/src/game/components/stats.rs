//! The numbers an entity fights by, worked out afresh every tick.

use bota_core::Odds;
use bota_proto::{Attribute, Attributes, DamageKind, Fixed};

/// Everything the type an entity is, its level, its items and what is on it
/// add up to.
///
/// Written by [`derive_stats`](crate::game::derive_stats), then by
/// [`World::guard_structures`](crate::game::World::guard_structures) for a
/// guarded structure; a value put in by hand is gone next tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    /// The three attributes, after everything that adds to them.
    pub attributes: Attributes,
    /// Which attribute pays its attack damage. Absent for whatever has none.
    pub primary: Option<Attribute>,
    pub max_hp: Fixed,
    pub max_mana: Fixed,
    /// Health mended each tick.
    pub hp_regen: Fixed,
    /// Mana mended each tick.
    pub mana_regen: Fixed,
    /// The least damage one attack deals.
    pub damage: i32,
    /// How much more one attack may deal, drawn evenly from `0..=` this on
    /// every swing.
    pub damage_spread: i32,
    /// Attack damage added against a creep and nothing else.
    pub damage_to_creeps: i32,
    /// How far it reaches, edge to edge.
    pub attack_range: Fixed,
    /// How far it looks for something to attack.
    pub acquisition: Fixed,
    /// Milliseconds between the starts of two attacks at the base attack
    /// speed.
    pub attack_time: u32,
    /// How fast it swings, where [`rules::BASE_ATTACK_SPEED`] is its own pace
    /// and twice that is twice the pace. Clamped to
    /// `MIN_ATTACK_SPEED..=MAX_ATTACK_SPEED` where read.
    ///
    /// [`rules::BASE_ATTACK_SPEED`]: bota_core::rules::BASE_ATTACK_SPEED
    pub attack_speed: i32,
    /// Milliseconds from the start of an attack to the hit at the base
    /// attack speed.
    pub attack_point: u32,
    /// Milliseconds after the hit before it may move again at the base
    /// attack speed.
    pub attack_backswing: u32,
    /// Speed of the missile it throws. Absent for a melee attack.
    pub projectile_speed: Option<Fixed>,
    pub armor: Fixed,
    /// Magic resistance, in basis points where 10_000 keeps everything out,
    /// within `rules::MIN_MAGIC_RESIST_BP..=10_000`. Below zero, magic
    /// damage lands amplified.
    pub magic_resist_bp: i32,
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
    /// Chance an attack at it misses: every source of it stacked as Dota
    /// stacks evasion.
    pub evasion: Odds,
    /// Percent of the physical damage its attacks deal healed back off a
    /// hero, and [`rules::LIFESTEAL_CREEP_PCT`] of it off anything else:
    /// every lifesteal, an item's or an ability's.
    ///
    /// [`rules::LIFESTEAL_CREEP_PCT`]: bota_core::rules::LIFESTEAL_CREEP_PCT
    pub lifesteal_pct: i32,
    /// Percent of the damage its spells deal healed back, and of the
    /// magical damage riding with its attacks, pure damage aside;
    /// [`rules::SPELL_LIFESTEAL_CREEP_PCT`] of it off anything but a hero.
    ///
    /// [`rules::SPELL_LIFESTEAL_CREEP_PCT`]: bota_core::rules::SPELL_LIFESTEAL_CREEP_PCT
    pub spell_lifesteal_pct: i32,
    /// Percent more damage its spells deal ([`Hit::spell`](crate::game::Hit::spell)), on top of
    /// every other amplification of what it deals.
    pub spell_amp_pct: i32,
    /// Percent added to how fast it casts: a cast point and a backswing take
    /// `100 / (100 + this)` of their length.
    pub cast_speed_pct: i32,
    /// Percent more magical damage it takes.
    pub magic_taken_pct: i32,
    /// Share of the damage that reaches it, after armor and resistance, that
    /// it takes, in basis points where 10_000 is all of it: an illusion
    /// takes more than that.
    pub damage_taken_bp: i32,
    /// Share of what it regenerates and of every heal it takes that comes
    /// through, in basis points where 10_000 is all of it: Dota's Health
    /// Restoration.
    pub restoration_bp: i32,
    pub illusion_attack_taken_bp: i32,
    pub slow_resist_bp: i32,
    /// Chance an attack of its goes through evasion: every source of it
    /// stacked as Dota stacks accuracy.
    pub accuracy: Odds,
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
    pub pit_blind: bool,
    pub ward_response: Option<bota_core::WardResponse>,
    /// Whether it flies: closed ground is nothing to it.
    pub flies: bool,
    pub tree_walking: bool,
    pub forced_move_immune: bool,
    /// Whether it walks through the bodies in its way.
    pub phased: bool,
    /// Whether damage passes it by.
    pub invulnerable: bool,
    /// Whether an enemy's spell passes it by unless the spell pierces it.
    pub spell_immune: bool,
    /// World units added to every radius of its that an AoE bonus widens: what it carries
    /// ([`Carried::aoe_bonus`](bota_core::Carried::aoe_bonus)).
    pub aoe_bonus: i32,
    /// World units added to the reach of its casts: what it carries
    /// ([`Carried::cast_range_bonus`](bota_core::Carried::cast_range_bonus)).
    pub cast_range_bonus: i32,
    /// Whether it is ethereal ([`ModifierKind::Ethereal`](bota_core::ModifierKind::Ethereal)):
    /// no attack lands on it or leaves it, and physical damage passes it by.
    pub ethereal: bool,
    /// Its health counted in hits, see [`UnitDef::hit_count`].
    ///
    /// [`UnitDef::hit_count`]: bota_core::UnitDef::hit_count
    pub hit_count: Option<bota_core::HitCount>,
    /// Whether the side's Glyph of Fortification holds on it: damage does
    /// nothing to it.
    pub glyph: bool,
    /// The side it sees for besides its own, and how far: what a
    /// [`ModifierKind::VisionGrant`](bota_core::ModifierKind::VisionGrant)
    /// on it gives.
    pub vision_grant: Option<(bota_proto::Team, Fixed)>,
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

    /// Its outgoing amplification of a kind of damage a spell of its deals, or magic riding with
    /// its attacks: the kind's with its spell amplification added (Dota's spell amplifications
    /// stack additively: an Amplify Damage Rune's 15% and a Kaya's 10% amplify by 25%), in basis
    /// points.
    pub fn spell_damage_amp_bp(&self, kind: DamageKind) -> i32 {
        self.damage_amp_bp(kind)
            .saturating_add(self.spell_amp_pct.saturating_mul(100))
            .max(0)
    }
}
