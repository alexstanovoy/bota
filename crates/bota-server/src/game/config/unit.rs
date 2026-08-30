//! What each kind of unit is worth before anything is done to it.
//!
//! Every entity that fights points at one of these. The numbers here are the
//! plain form of its type; level, upgrades, items and whatever is on it are
//! added by the system that works out [`Stats`].
//!
//! [`Stats`]: crate::game::Stats

use bota_proto::{Attribute, Attributes, Fixed, UnitKind};

use crate::game::rules;
use crate::game::{Aura, StatusKind};

/// What an entity gains for each step of whatever raises it: a level past the
/// first, or an upgrade interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Growth {
    /// Attributes added.
    pub attributes: Attributes,
    /// Health added.
    pub hp: i32,
    /// Mana added.
    pub mana: i32,
    /// Attack damage added.
    pub damage: i32,
    /// Half points of armor added, so an odd number is half a point a step.
    pub armor_halves: i32,
    /// Gold added to the bounty.
    pub gold: i32,
    /// Experience added to the bounty.
    pub xp: i32,
}

/// The plain form of one kind of unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitDef {
    /// What kind of thing it is.
    pub kind: UnitKind,
    /// Attributes it spawns with. All zero for whatever has none.
    pub attributes: Attributes,
    /// Which attribute pays its attack damage. Absent for whatever has none.
    pub primary: Option<Attribute>,
    /// Health it spawns with, before strength.
    pub max_hp: i32,
    /// Mana it spawns with, before intelligence. Zero for whatever casts
    /// nothing.
    pub max_mana: i32,
    /// Health mended each tick.
    pub hp_regen: Fixed,
    /// Mana mended each tick.
    pub mana_regen: Fixed,
    /// Damage one attack deals, before the primary attribute. Zero for
    /// whatever does not attack.
    pub damage: i32,
    /// How far it reaches, edge to edge, in world units.
    pub attack_range: i32,
    /// How far it looks for something to attack, in world units.
    pub acquisition: i32,
    /// Ticks between the starts of two attacks at [`rules::BASE_ATTACK_SPEED`].
    pub attack_interval: u32,
    /// Ticks from the start of an attack to the hit.
    pub attack_point: u32,
    /// Ticks after the hit before it may move again.
    pub attack_backswing: u32,
    /// Speed of the missile it throws, in world units per second. Absent for a
    /// melee attack.
    pub projectile_speed: Option<i32>,
    /// Armor, reducing physical damage, before agility.
    pub armor: i32,
    /// Magic resistance, percent.
    pub magic_resist_pct: i32,
    /// World units per second on the ground. Zero for whatever cannot walk.
    pub move_speed: i32,
    /// Brads per tick it turns.
    pub turn_rate: u16,
    /// How far it sees, in world units.
    pub vision: i32,
    /// How far it reveals what hides, in world units. Zero for whatever gives
    /// no true sight.
    pub true_sight: i32,
    /// Whether the other side sees it only through true sight.
    pub hides: bool,
    /// Whether it flies: closed ground is nothing to it.
    pub flies: bool,
    /// Whether it only carries for another: what is in its bag is worth
    /// nothing to it.
    pub porter: bool,
    /// The circle it occupies, in world units.
    pub radius: i32,
    /// Whether damage passes it by.
    pub invulnerable: bool,
    /// Whether it counts as ancient.
    pub ancient: bool,
    /// Gold killing it pays.
    pub bounty_gold: i32,
    /// Experience killing it pays.
    pub bounty_xp: i32,
    /// What each level past the first adds.
    pub per_level: Growth,
    /// What each upgrade interval adds.
    pub per_upgrade: Growth,
    /// What it hands out to its own side for standing near it.
    pub auras: &'static [Aura],
}

/// No gain at all.
const NO_GROWTH: Growth = Growth {
    attributes: Attributes::ZERO,
    hp: 0,
    mana: 0,
    damage: 0,
    armor_halves: 0,
    gold: 0,
    xp: 0,
};

/// Nothing at all, so a definition names only the fields that differ.
const NOTHING: UnitDef = UnitDef {
    kind: UnitKind::Ward,
    attributes: Attributes::ZERO,
    primary: None,
    max_hp: 0,
    max_mana: 0,
    hp_regen: Fixed::ZERO,
    mana_regen: Fixed::ZERO,
    damage: 0,
    attack_range: 0,
    acquisition: 0,
    attack_interval: 0,
    attack_point: 0,
    attack_backswing: 0,
    projectile_speed: None,
    armor: 0,
    magic_resist_pct: 0,
    move_speed: 0,
    turn_rate: 0,
    vision: 0,
    true_sight: 0,
    hides: false,
    flies: false,
    porter: false,
    radius: 0,
    invulnerable: false,
    ancient: false,
    bounty_gold: 0,
    bounty_xp: 0,
    per_level: NO_GROWTH,
    per_upgrade: NO_GROWTH,
    auras: &[],
};

/// Which kind of unit an entity is. Points into the table, never a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Def(pub &'static UnitDef);

/// A melee lane creep.
pub const MELEE_CREEP: UnitDef = UnitDef {
    kind: UnitKind::CreepMelee,
    max_hp: rules::MELEE_CREEP_HP,
    damage: rules::MELEE_CREEP_ATTACK_DAMAGE,
    attack_range: rules::MELEE_CREEP_ATTACK_RANGE,
    acquisition: rules::MELEE_CREEP_ACQUISITION,
    attack_interval: rules::CREEP_ATTACK_INTERVAL,
    attack_point: rules::MELEE_CREEP_ATTACK_POINT,
    attack_backswing: rules::CREEP_ATTACK_BACKSWING,
    armor: rules::MELEE_CREEP_ARMOR,
    move_speed: rules::CREEP_MOVE_SPEED,
    turn_rate: rules::TURN_RATE_BRADS,
    vision: rules::CREEP_VISION,
    radius: rules::MELEE_CREEP_RADIUS,
    bounty_gold: rules::MELEE_CREEP_BOUNTY,
    bounty_xp: rules::MELEE_CREEP_XP,
    per_upgrade: Growth {
        hp: rules::MELEE_UPGRADE_HP,
        damage: rules::MELEE_UPGRADE_DAMAGE,
        gold: rules::MELEE_UPGRADE_GOLD,
        ..NO_GROWTH
    },
    ..NOTHING
};

/// A melee lane creep carrying the flag. Takes no upgrades.
pub const FLAGBEARER_CREEP: UnitDef = UnitDef {
    kind: UnitKind::CreepFlagbearer,
    magic_resist_pct: rules::FLAGBEARER_MAGIC_RESIST_PCT,
    per_upgrade: NO_GROWTH,
    ..MELEE_CREEP
};

/// A ranged lane creep.
pub const RANGED_CREEP: UnitDef = UnitDef {
    kind: UnitKind::CreepRanged,
    max_hp: rules::RANGED_CREEP_HP,
    damage: rules::RANGED_CREEP_ATTACK_DAMAGE,
    attack_range: rules::RANGED_CREEP_ATTACK_RANGE,
    acquisition: rules::RANGED_CREEP_ACQUISITION,
    attack_point: rules::RANGED_CREEP_ATTACK_POINT,
    projectile_speed: Some(rules::RANGED_CREEP_PROJECTILE_SPEED),
    radius: rules::RANGED_CREEP_RADIUS,
    bounty_gold: rules::RANGED_CREEP_BOUNTY,
    bounty_xp: rules::RANGED_CREEP_XP,
    per_upgrade: Growth {
        hp: rules::RANGED_UPGRADE_HP,
        damage: rules::RANGED_UPGRADE_DAMAGE,
        gold: rules::RANGED_UPGRADE_GOLD,
        xp: rules::RANGED_UPGRADE_XP,
        ..NO_GROWTH
    },
    ..MELEE_CREEP
};

/// A super melee creep: what spawns once the enemy melee barracks of the
/// lane has fallen.
pub const SUPER_MELEE_CREEP: UnitDef = UnitDef {
    max_hp: rules::SUPER_MELEE_HP,
    damage: rules::SUPER_MELEE_ATTACK_DAMAGE,
    armor: rules::SUPER_MELEE_ARMOR,
    bounty_gold: rules::SUPER_MELEE_BOUNTY,
    bounty_xp: rules::SUPER_MELEE_XP,
    ..MELEE_CREEP
};

/// A mega melee creep: a super one swinging faster, once every enemy
/// barracks has fallen.
pub const MEGA_MELEE_CREEP: UnitDef = UnitDef {
    attack_interval: rules::MEGA_MELEE_ATTACK_INTERVAL,
    ..SUPER_MELEE_CREEP
};

/// A super ranged creep.
pub const SUPER_RANGED_CREEP: UnitDef = UnitDef {
    max_hp: rules::SUPER_RANGED_HP,
    damage: rules::SUPER_RANGED_ATTACK_DAMAGE,
    armor: rules::SUPER_RANGED_ARMOR,
    bounty_gold: rules::SUPER_RANGED_BOUNTY,
    bounty_xp: rules::SUPER_RANGED_XP,
    ..RANGED_CREEP
};

/// A siege creep. Takes no upgrades.
pub const SIEGE_CREEP: UnitDef = UnitDef {
    kind: UnitKind::CreepSiege,
    max_hp: rules::SIEGE_CREEP_HP,
    damage: rules::SIEGE_CREEP_ATTACK_DAMAGE,
    attack_range: rules::SIEGE_CREEP_ATTACK_RANGE,
    acquisition: rules::SIEGE_CREEP_ACQUISITION,
    attack_interval: rules::SIEGE_CREEP_ATTACK_INTERVAL,
    attack_point: rules::SIEGE_CREEP_ATTACK_POINT,
    projectile_speed: Some(rules::SIEGE_CREEP_PROJECTILE_SPEED),
    armor: rules::SIEGE_CREEP_ARMOR,
    magic_resist_pct: rules::SIEGE_CREEP_MAGIC_RESIST_PCT,
    radius: rules::SIEGE_CREEP_RADIUS,
    bounty_gold: rules::SIEGE_CREEP_BOUNTY,
    bounty_xp: rules::SIEGE_CREEP_XP,
    per_upgrade: NO_GROWTH,
    ..MELEE_CREEP
};

/// A hero at level one.
pub const HERO: UnitDef = UnitDef {
    kind: UnitKind::Hero,
    attributes: rules::HERO_ATTRIBUTES,
    primary: Some(Attribute::Agility),
    max_hp: rules::HERO_HP,
    max_mana: rules::HERO_MANA,
    hp_regen: rules::HERO_HP_REGEN,
    mana_regen: rules::HERO_MANA_REGEN,
    damage: rules::HERO_ATTACK_DAMAGE,
    attack_range: rules::HERO_ATTACK_RANGE,
    acquisition: rules::ACQUISITION_RANGE,
    attack_interval: rules::HERO_ATTACK_INTERVAL,
    attack_point: rules::HERO_ATTACK_POINT,
    attack_backswing: rules::HERO_ATTACK_BACKSWING,
    projectile_speed: Some(rules::HERO_PROJECTILE_SPEED),
    armor: rules::HERO_ARMOR,
    magic_resist_pct: rules::HERO_MAGIC_RESIST_PCT,
    move_speed: rules::HERO_MOVE_SPEED,
    turn_rate: rules::TURN_RATE_BRADS,
    vision: rules::HERO_VISION,
    radius: rules::HERO_RADIUS,
    per_level: Growth {
        attributes: rules::HERO_ATTRIBUTES_PER_LEVEL,
        hp: rules::HERO_HP_PER_LEVEL,
        mana: rules::HERO_MANA_PER_LEVEL,
        damage: rules::HERO_ATTACK_DAMAGE_PER_LEVEL,
        ..NO_GROWTH
    },
    ..NOTHING
};

/// Pudge: heavy, slow, and swings by hand.
pub const PUDGE: UnitDef = UnitDef {
    attributes: Attributes {
        strength: Fixed::from_int(25),
        agility: Fixed::from_int(14),
        intelligence: Fixed::from_int(14),
    },
    primary: Some(Attribute::Strength),
    max_hp: 150,
    max_mana: 82,
    damage: 21,
    attack_range: 150,
    attack_interval: 58,
    attack_point: 15,
    projectile_speed: None,
    armor: -1,
    move_speed: 280,
    per_level: Growth {
        attributes: Attributes {
            strength: Fixed::from_ratio(35, 10),
            agility: Fixed::from_ratio(15, 10),
            intelligence: Fixed::from_ratio(16, 10),
        },
        hp: 43,
        mana: 5,
        damage: 1,
        ..NO_GROWTH
    },
    ..HERO
};

/// Shadow Fiend: fragile, long-ranged, and everything it grows into is in
/// its attack.
pub const SHADOW_FIEND: UnitDef = UnitDef {
    attributes: Attributes {
        strength: Fixed::from_int(18),
        agility: Fixed::from_int(20),
        intelligence: Fixed::from_int(18),
    },
    primary: Some(Attribute::Agility),
    max_hp: 120,
    max_mana: 75,
    damage: 25,
    attack_range: 500,
    attack_interval: 51,
    attack_point: 15,
    projectile_speed: Some(1200),
    armor: 0,
    move_speed: 305,
    per_level: Growth {
        attributes: Attributes {
            strength: Fixed::from_ratio(20, 10),
            agility: Fixed::from_ratio(33, 10),
            intelligence: Fixed::from_ratio(21, 10),
        },
        hp: 38,
        mana: 6,
        damage: 1,
        ..NO_GROWTH
    },
    ..HERO
};

/// Roshan.
pub const ROSHAN: UnitDef = UnitDef {
    kind: UnitKind::Roshan,
    max_hp: rules::ROSHAN_HP,
    damage: rules::ROSHAN_ATTACK_DAMAGE,
    attack_range: rules::ROSHAN_ATTACK_RANGE,
    acquisition: rules::ROSHAN_ATTACK_RANGE,
    attack_interval: rules::ROSHAN_ATTACK_INTERVAL,
    attack_point: rules::ROSHAN_ATTACK_POINT,
    armor: rules::ROSHAN_ARMOR,
    magic_resist_pct: rules::ROSHAN_MAGIC_RESIST_PCT,
    move_speed: rules::ROSHAN_MOVE_SPEED,
    turn_rate: rules::TURN_RATE_BRADS,
    vision: rules::ROSHAN_VISION,
    radius: rules::ROSHAN_RADIUS,
    bounty_gold: rules::ROSHAN_BOUNTY,
    bounty_xp: rules::ROSHAN_XP,
    ..NOTHING
};

/// A super siege creep: the same wagon hitting harder.
pub const SUPER_SIEGE_CREEP: UnitDef = UnitDef {
    damage: rules::SUPER_SIEGE_ATTACK_DAMAGE,
    ..SIEGE_CREEP
};

/// A melee barracks.
pub const BARRACKS_MELEE: UnitDef = UnitDef {
    kind: UnitKind::Barracks,
    max_hp: rules::RAX_MELEE_HP,
    hp_regen: rules::RAX_MELEE_HP_REGEN,
    armor: rules::RAX_MELEE_ARMOR,
    vision: rules::RAX_VISION,
    radius: rules::RAX_RADIUS,
    bounty_gold: rules::RAX_MELEE_BOUNTY,
    ..NOTHING
};

/// A ranged barracks.
pub const BARRACKS_RANGED: UnitDef = UnitDef {
    kind: UnitKind::Barracks,
    max_hp: rules::RAX_RANGED_HP,
    armor: rules::RAX_RANGED_ARMOR,
    vision: rules::RAX_VISION,
    radius: rules::RAX_RADIUS,
    bounty_gold: rules::RAX_RANGED_BOUNTY,
    ..NOTHING
};

/// An Ancient.
pub const ANCIENT: UnitDef = UnitDef {
    kind: UnitKind::Ancient,
    max_hp: rules::ANCIENT_HP,
    armor: rules::ANCIENT_ARMOR,
    vision: rules::ANCIENT_VISION,
    radius: rules::ANCIENT_RADIUS,
    ..NOTHING
};

/// A fountain.
/// What a fountain mends on its own side standing in it.
const FOUNTAIN_AURAS: [Aura; 1] = [Aura {
    kind: StatusKind::Fountain {
        hp_per_tick: rules::FOUNTAIN_HEAL_HP_PER_TICK * 100,
        mana_per_tick: rules::FOUNTAIN_HEAL_MANA_PER_TICK * 100,
    },
    radius: rules::FOUNTAIN_HEAL_RADIUS,
    ticks: rules::TICKS_PER_SECOND,
}];

pub const FOUNTAIN: UnitDef = UnitDef {
    kind: UnitKind::Fountain,
    max_hp: rules::FOUNTAIN_HP,
    auras: &FOUNTAIN_AURAS,
    damage: rules::FOUNTAIN_ATTACK_DAMAGE,
    attack_range: rules::FOUNTAIN_ATTACK_RANGE,
    acquisition: rules::FOUNTAIN_ATTACK_RANGE,
    attack_interval: rules::FOUNTAIN_ATTACK_INTERVAL,
    attack_point: rules::FOUNTAIN_ATTACK_POINT,
    attack_backswing: rules::FOUNTAIN_ATTACK_BACKSWING,
    projectile_speed: Some(rules::FOUNTAIN_PROJECTILE_SPEED),
    vision: rules::FOUNTAIN_VISION,
    radius: rules::FOUNTAIN_RADIUS,
    invulnerable: true,
    ..NOTHING
};

/// A courier: it carries, it does not fight, and it flies over everything.
pub const COURIER: UnitDef = UnitDef {
    kind: UnitKind::Courier,
    flies: true,
    porter: true,
    magic_resist_pct: 100,
    max_hp: rules::COURIER_HP,
    move_speed: rules::COURIER_MOVE_SPEED,
    turn_rate: rules::TURN_RATE_BRADS,
    vision: rules::COURIER_VISION,
    ..NOTHING
};

/// An observer ward: it sees far and the other side cannot see it.
pub const OBSERVER_WARD: UnitDef = UnitDef {
    kind: UnitKind::Ward,
    max_hp: 200,
    vision: 1600,
    hides: true,
    ..NOTHING
};

/// A sentry ward: it sees nothing of itself, and what it gives is true sight.
pub const SENTRY_WARD: UnitDef = UnitDef {
    kind: UnitKind::Ward,
    max_hp: 200,
    vision: 0,
    true_sight: 850,
    hides: true,
    ..NOTHING
};

/// A lane tower of one tier.
const fn tower_of(index: usize) -> UnitDef {
    UnitDef {
        kind: UnitKind::Tower,
        true_sight: rules::TOWER_ATTACK_RANGE,
        max_hp: rules::TOWER_TIER_HP[index],
        damage: rules::TOWER_TIER_DAMAGE[index],
        attack_range: rules::TOWER_ATTACK_RANGE,
        acquisition: rules::TOWER_ATTACK_RANGE,
        attack_interval: rules::TOWER_ATTACK_INTERVAL,
        attack_point: rules::TOWER_ATTACK_POINT,
        attack_backswing: rules::TOWER_ATTACK_BACKSWING,
        projectile_speed: Some(rules::TOWER_PROJECTILE_SPEED),
        armor: rules::TOWER_TIER_ARMOR[index],
        turn_rate: rules::TURN_RATE_BRADS,
        vision: rules::TOWER_VISION,
        radius: rules::TOWER_RADIUS,
        bounty_gold: rules::TOWER_TIER_BOUNTY[index],
        ..NOTHING
    }
}

/// Lane towers, indexed by tier less one.
pub const TOWERS: [UnitDef; 4] = [tower_of(0), tower_of(1), tower_of(2), tower_of(3)];

/// The plain form of a tower of this tier, counted from one.
pub fn tower_def(tier: u8) -> &'static UnitDef {
    &TOWERS[usize::from(tier.clamp(1, 4)) - 1]
}

/// Whether a kind of unit is a building: it stands still, blocks the ground,
/// and both sides always know where it is.
pub fn is_structure(kind: UnitKind) -> bool {
    matches!(
        kind,
        UnitKind::Tower | UnitKind::Ancient | UnitKind::Barracks | UnitKind::Fountain
    )
}

/// Whether a kind of unit leaves anything to what is gathered from deaths:
/// the flesh heap and the souls. A structure or a ward leaves nothing.
pub fn leaves_a_death(kind: UnitKind) -> bool {
    !is_structure(kind) && kind != UnitKind::Ward
}

/// Whether a kind of unit is a lane creep: one of what a wave is made of.
pub fn is_lane_creep(kind: UnitKind) -> bool {
    matches!(
        kind,
        UnitKind::CreepMelee
            | UnitKind::CreepFlagbearer
            | UnitKind::CreepRanged
            | UnitKind::CreepSiege
    )
}

/// Whether a kind of unit is a creep: what a wave is made of, and what the
/// jungle grows.
pub fn is_creep(kind: UnitKind) -> bool {
    is_lane_creep(kind) || kind == UnitKind::CreepNeutral
}
