//! What each kind of unit is worth before anything is done to it.
//!
//! Every entity that fights points at one of these. The numbers here are the
//! plain form of its type; level, upgrades, items and whatever is on it are
//! added by the server as it works out what a unit fights by.

use bota_proto::{AbilityId, Attribute, Attributes, Fixed, Team, UnitKind};

use crate::{Aura, AuraModifier, GoldRange, ModifierKind, Reach, Whom, rules};

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
    /// Attack speed added.
    pub attack_speed: i32,
    /// Half points of gold added to both ends of the bounty, so an odd
    /// number is half a gold a step; a part gold left over is not paid.
    pub gold_halves: i32,
    /// Experience added to the bounty.
    pub xp: i32,
}

/// Where a building's backdoor protection looks for enemy lane creeps:
/// Dota's `backdoor_protection` and `backdoor_protection_in_base`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackdoorWatch {
    /// Round itself, out to `radius` world units: a tier two tower's own.
    Own { radius: i32 },
    /// Round itself, out to `radius` world units, for its whole base: the
    /// Ancient's, which every [`InBase`](Self::InBase) building of its side
    /// holds by.
    Base { radius: i32 },
    /// Through its side's [`Base`](Self::Base) watch.
    InBase,
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
    /// The least damage one attack deals, before the primary attribute.
    /// Zero for whatever does not attack.
    pub damage: i32,
    /// How much more one attack may deal: each swing adds a whole number
    /// drawn evenly from `0..=damage_spread`. Zero for a fixed amount.
    pub damage_spread: i32,
    /// How far it reaches, edge to edge, in world units.
    pub attack_range: i32,
    /// How far it looks for something to attack, in world units.
    pub acquisition: i32,
    /// Milliseconds between the starts of two attacks at
    /// [`rules::BASE_ATTACK_SPEED`].
    pub attack_time: u32,
    /// The attack speed it has before anything raises it.
    pub attack_speed: i32,
    /// Milliseconds from the start of an attack to the hit.
    pub attack_point: u32,
    /// Milliseconds after the hit before it may move again.
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
    /// How far it sees by day, in world units.
    pub day_vision: i32,
    /// How far it sees by night, in world units.
    pub night_vision: i32,
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
    /// Whether its pace knows no upper limit, as Dota's courier's does: no
    /// haste of it stops at [`rules::MAX_MOVE_SPEED`].
    pub unbounded_pace: bool,
    /// Collision size: how near another body's centre may come, less that
    /// body's own, in world units. Zero for what has no body.
    pub collision: i32,
    /// Bound radius: where its edge is for attack range, cast range and
    /// areas, in world units.
    pub bound: i32,
    /// Whether damage passes it by.
    pub invulnerable: bool,
    /// Whether it counts as ancient.
    pub ancient: bool,
    /// Gold killing it pays.
    pub bounty_gold: GoldRange,
    /// Gold its fall pays every seat of the side that brought it down, half
    /// to every seat of both sides on a deny. Zero for whatever pays none.
    pub bounty_team: i32,
    /// Experience killing it pays.
    pub bounty_xp: i32,
    /// What each level past the first adds.
    pub per_level: Growth,
    /// What each upgrade interval adds.
    pub per_upgrade: Growth,
    /// The auras it carries of its own, beside those of its abilities.
    pub auras: &'static [Aura],
    /// Its backdoor protection. Absent for whatever has none.
    pub backdoor: Option<BackdoorWatch>,
}

/// No gain at all.
pub(crate) const NO_GROWTH: Growth = Growth {
    attributes: Attributes::ZERO,
    hp: 0,
    mana: 0,
    damage: 0,
    armor_halves: 0,
    attack_speed: 0,
    gold_halves: 0,
    xp: 0,
};

/// Nothing at all, so a definition names only the fields that differ.
pub(crate) const BARE_UNIT: UnitDef = UnitDef {
    kind: UnitKind::Ward,
    attributes: Attributes::ZERO,
    primary: None,
    max_hp: 0,
    max_mana: 0,
    hp_regen: Fixed::ZERO,
    mana_regen: Fixed::ZERO,
    damage: 0,
    damage_spread: 0,
    attack_range: 0,
    acquisition: 0,
    attack_time: 0,
    attack_speed: rules::BASE_ATTACK_SPEED,
    attack_point: 0,
    attack_backswing: 0,
    projectile_speed: None,
    armor: 0,
    magic_resist_pct: 0,
    move_speed: 0,
    turn_rate: 0,
    day_vision: 0,
    night_vision: 0,
    true_sight: 0,
    hides: false,
    flies: false,
    porter: false,
    unbounded_pace: false,
    collision: 0,
    bound: 0,
    invulnerable: false,
    ancient: false,
    bounty_gold: GoldRange::ZERO,
    bounty_team: 0,
    bounty_xp: 0,
    per_level: NO_GROWTH,
    per_upgrade: NO_GROWTH,
    auras: &[],
    backdoor: None,
};

// The lane creeps' numbers are the game's `scripts/npc/npc_units.txt`
// (client 6874, 2026-07-31) and the creep abilities of its
// `npc_abilities.txt`, checked against Liquipedia's *Lane Creeps* (oldid
// 2401399, read 2026-10-10) and *Flagbearer Creep* (oldid 2364908) and dated
// by Valve's patch notes 7.39 to 7.41; hull radii from the Valve Developer
// Community's *BoundsHullName Size Reference*.

/// A melee lane creep.
pub const MELEE_CREEP: UnitDef = UnitDef {
    kind: UnitKind::CreepMelee,
    max_hp: rules::MELEE_CREEP_HP,
    damage: rules::MELEE_CREEP_ATTACK_DAMAGE,
    damage_spread: rules::MELEE_CREEP_DAMAGE_SPREAD,
    attack_range: rules::MELEE_CREEP_ATTACK_RANGE,
    acquisition: rules::MELEE_CREEP_ACQUISITION,
    attack_time: rules::CREEP_ATTACK_TIME,
    attack_point: rules::MELEE_CREEP_ATTACK_POINT,
    attack_backswing: rules::CREEP_ATTACK_BACKSWING,
    armor: rules::MELEE_CREEP_ARMOR,
    move_speed: rules::CREEP_MOVE_SPEED,
    turn_rate: rules::TURN_RATE_BRADS,
    day_vision: rules::CREEP_VISION,
    night_vision: rules::CREEP_VISION,
    collision: rules::MELEE_CREEP_COLLISION,
    bound: rules::MELEE_CREEP_BOUND,
    bounty_gold: rules::MELEE_CREEP_BOUNTY,
    bounty_xp: rules::MELEE_CREEP_XP,
    hp_regen: rules::MELEE_CREEP_HP_REGEN,
    per_upgrade: Growth {
        hp: rules::MELEE_UPGRADE_HP,
        damage: rules::MELEE_UPGRADE_DAMAGE,
        gold_halves: 2 * rules::MELEE_UPGRADE_GOLD,
        ..NO_GROWTH
    },
    ..BARE_UNIT
};

/// What a flagbearer mends in the creeps marching with it, and the magic
/// resistance it grows into with the upgrade intervals it spawned after:
/// Dota's `flagbearer_creep_aura_effect`, a passive a Break silences.
const FLAGBEARER_AURAS: [Aura; 1] = [Aura {
    whom: Whom::Allies,
    reaches: Reach::Units,
    radius: rules::FLAGBEARER_AURA_RADIUS,
    linger: rules::AURA_LINGER_TICKS,
    breakable: true,
    seen_only: false,
    water: false,
    modifier: AuraModifier::Kind(ModifierKind::Inspired {
        hp_per_second: rules::FLAGBEARER_AURA_REGEN,
        magic_resist_pct: 0,
    }),
}];

/// A melee lane creep carrying the flag. Upgrades as a melee creep does.
pub const FLAGBEARER_CREEP: UnitDef = UnitDef {
    kind: UnitKind::CreepFlagbearer,
    magic_resist_pct: rules::FLAGBEARER_MAGIC_RESIST_PCT,
    bounty_xp: rules::FLAGBEARER_XP,
    auras: &FLAGBEARER_AURAS,
    ..MELEE_CREEP
};

/// A ranged lane creep.
pub const RANGED_CREEP: UnitDef = UnitDef {
    kind: UnitKind::CreepRanged,
    max_hp: rules::RANGED_CREEP_HP,
    damage: rules::RANGED_CREEP_ATTACK_DAMAGE,
    damage_spread: rules::RANGED_CREEP_DAMAGE_SPREAD,
    attack_range: rules::RANGED_CREEP_ATTACK_RANGE,
    acquisition: rules::RANGED_CREEP_ACQUISITION,
    attack_point: rules::RANGED_CREEP_ATTACK_POINT,
    projectile_speed: Some(rules::RANGED_CREEP_PROJECTILE_SPEED),
    collision: rules::RANGED_CREEP_COLLISION,
    bound: rules::RANGED_CREEP_BOUND,
    bounty_gold: rules::RANGED_CREEP_BOUNTY,
    bounty_xp: rules::RANGED_CREEP_XP,
    hp_regen: rules::RANGED_CREEP_HP_REGEN,
    per_upgrade: Growth {
        hp: rules::RANGED_UPGRADE_HP,
        damage: rules::RANGED_UPGRADE_DAMAGE,
        gold_halves: 2 * rules::RANGED_UPGRADE_GOLD,
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
    damage_spread: rules::SUPER_MELEE_DAMAGE_SPREAD,
    armor: rules::SUPER_MELEE_ARMOR,
    bounty_gold: rules::SUPER_MELEE_BOUNTY,
    bounty_xp: rules::SUPER_MELEE_XP,
    per_upgrade: Growth {
        hp: rules::SUPER_MELEE_UPGRADE_HP,
        damage: rules::SUPER_MELEE_UPGRADE_DAMAGE,
        gold_halves: rules::SUPER_MELEE_UPGRADE_GOLD_HALVES,
        ..NO_GROWTH
    },
    ..MELEE_CREEP
};

/// A mega melee creep, once every enemy barracks has fallen: a fully
/// upgraded super one whose bounty alone still grows.
pub const MEGA_MELEE_CREEP: UnitDef = UnitDef {
    max_hp: rules::MEGA_MELEE_HP,
    damage: rules::MEGA_MELEE_ATTACK_DAMAGE,
    per_upgrade: Growth {
        gold_halves: rules::SUPER_MELEE_UPGRADE_GOLD_HALVES,
        ..NO_GROWTH
    },
    ..SUPER_MELEE_CREEP
};

pub const SUPER_RANGED_CREEP: UnitDef = UnitDef {
    max_hp: rules::SUPER_RANGED_HP,
    damage: rules::SUPER_RANGED_ATTACK_DAMAGE,
    damage_spread: rules::SUPER_RANGED_DAMAGE_SPREAD,
    armor: rules::SUPER_RANGED_ARMOR,
    bounty_gold: rules::SUPER_RANGED_BOUNTY,
    bounty_xp: rules::SUPER_RANGED_XP,
    per_upgrade: Growth {
        hp: rules::SUPER_RANGED_UPGRADE_HP,
        damage: rules::SUPER_RANGED_UPGRADE_DAMAGE,
        gold_halves: 2 * rules::SUPER_RANGED_UPGRADE_GOLD,
        ..NO_GROWTH
    },
    ..RANGED_CREEP
};

/// A mega ranged creep, once every enemy barracks has fallen: a fully
/// upgraded super one whose bounty alone still grows.
pub const MEGA_RANGED_CREEP: UnitDef = UnitDef {
    max_hp: rules::MEGA_RANGED_HP,
    damage: rules::MEGA_RANGED_ATTACK_DAMAGE,
    per_upgrade: Growth {
        gold_halves: 2 * rules::SUPER_RANGED_UPGRADE_GOLD,
        ..NO_GROWTH
    },
    ..SUPER_RANGED_CREEP
};

/// A siege creep. Takes no upgrades.
pub const SIEGE_CREEP: UnitDef = UnitDef {
    kind: UnitKind::CreepSiege,
    max_hp: rules::SIEGE_CREEP_HP,
    damage: rules::SIEGE_CREEP_ATTACK_DAMAGE,
    damage_spread: rules::SIEGE_CREEP_DAMAGE_SPREAD,
    attack_range: rules::SIEGE_CREEP_ATTACK_RANGE,
    acquisition: rules::SIEGE_CREEP_ACQUISITION,
    attack_time: rules::SIEGE_CREEP_ATTACK_TIME,
    attack_point: rules::SIEGE_CREEP_ATTACK_POINT,
    projectile_speed: Some(rules::SIEGE_CREEP_PROJECTILE_SPEED),
    armor: rules::SIEGE_CREEP_ARMOR,
    magic_resist_pct: rules::SIEGE_CREEP_MAGIC_RESIST_PCT,
    collision: rules::SIEGE_CREEP_COLLISION,
    bound: rules::SIEGE_CREEP_BOUND,
    bounty_gold: rules::SIEGE_CREEP_BOUNTY,
    bounty_xp: rules::SIEGE_CREEP_XP,
    hp_regen: Fixed::ZERO,
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
    attack_time: rules::HERO_ATTACK_TIME,
    attack_point: rules::HERO_ATTACK_POINT,
    attack_backswing: rules::HERO_ATTACK_BACKSWING,
    projectile_speed: Some(rules::HERO_PROJECTILE_SPEED),
    armor: rules::HERO_ARMOR,
    magic_resist_pct: rules::HERO_MAGIC_RESIST_PCT,
    move_speed: rules::HERO_MOVE_SPEED,
    turn_rate: rules::TURN_RATE_BRADS,
    day_vision: rules::HERO_DAY_VISION,
    night_vision: rules::HERO_NIGHT_VISION,
    collision: rules::HERO_COLLISION,
    bound: rules::HERO_BOUND,
    per_level: Growth {
        attributes: rules::HERO_ATTRIBUTES_PER_LEVEL,
        hp: rules::HERO_HP_PER_LEVEL,
        mana: rules::HERO_MANA_PER_LEVEL,
        damage: rules::HERO_ATTACK_DAMAGE_PER_LEVEL,
        ..NO_GROWTH
    },
    ..BARE_UNIT
};

/// A super siege creep: the same wagon hitting harder.
pub const SUPER_SIEGE_CREEP: UnitDef = UnitDef {
    damage: rules::SUPER_SIEGE_ATTACK_DAMAGE,
    damage_spread: rules::SUPER_SIEGE_DAMAGE_SPREAD,
    ..SIEGE_CREEP
};

pub const BARRACKS_MELEE: UnitDef = UnitDef {
    kind: UnitKind::Barracks,
    max_hp: rules::RAX_MELEE_HP,
    hp_regen: rules::RAX_MELEE_HP_REGEN,
    armor: rules::RAX_MELEE_ARMOR,
    day_vision: rules::RAX_DAY_VISION,
    night_vision: rules::RAX_NIGHT_VISION,
    collision: rules::RAX_COLLISION,
    bound: rules::RAX_BOUND,
    bounty_gold: rules::RAX_BOUNTY,
    bounty_team: rules::RAX_MELEE_TEAM_BOUNTY,
    backdoor: Some(BackdoorWatch::InBase),
    ..BARE_UNIT
};

pub const BARRACKS_RANGED: UnitDef = UnitDef {
    kind: UnitKind::Barracks,
    max_hp: rules::RAX_RANGED_HP,
    armor: rules::RAX_RANGED_ARMOR,
    day_vision: rules::RAX_DAY_VISION,
    night_vision: rules::RAX_NIGHT_VISION,
    collision: rules::RAX_COLLISION,
    bound: rules::RAX_BOUND,
    bounty_gold: rules::RAX_BOUNTY,
    bounty_team: rules::RAX_RANGED_TEAM_BOUNTY,
    backdoor: Some(BackdoorWatch::InBase),
    ..BARE_UNIT
};

/// A filler statue: it pays its bounty when it falls and does nothing else.
pub const FILLER: UnitDef = UnitDef {
    kind: UnitKind::Filler,
    max_hp: rules::FILLER_HP,
    armor: rules::FILLER_ARMOR,
    day_vision: rules::FILLER_DAY_VISION,
    night_vision: rules::FILLER_NIGHT_VISION,
    collision: rules::FILLER_COLLISION,
    bound: rules::FILLER_BOUND,
    bounty_gold: GoldRange::exactly(rules::FILLER_BOUNTY),
    backdoor: Some(BackdoorWatch::InBase),
    ..BARE_UNIT
};

pub const RADIANT_ANCIENT: UnitDef = UnitDef {
    kind: UnitKind::Ancient,
    max_hp: rules::ANCIENT_HP,
    hp_regen: rules::ANCIENT_HP_REGEN,
    armor: rules::ANCIENT_ARMOR,
    day_vision: rules::ANCIENT_VISION,
    night_vision: rules::ANCIENT_VISION,
    collision: rules::RADIANT_ANCIENT_COLLISION,
    bound: rules::RADIANT_ANCIENT_BOUND,
    backdoor: Some(BackdoorWatch::Base {
        radius: rules::BACKDOOR_BASE_RADIUS,
    }),
    ..BARE_UNIT
};

/// The Dire Ancient: the same building on a wider footprint.
pub const DIRE_ANCIENT: UnitDef = UnitDef {
    collision: rules::DIRE_ANCIENT_COLLISION,
    bound: rules::DIRE_ANCIENT_BOUND,
    ..RADIANT_ANCIENT
};

/// The Ancient a side raises. The jungle raises the Radiant one.
pub fn ancient_of(team: Team) -> &'static UnitDef {
    match team {
        Team::Radiant | Team::Neutral => &RADIANT_ANCIENT,
        Team::Dire => &DIRE_ANCIENT,
    }
}

/// What a fountain mends on its own side standing in it: Dota's
/// `modifier_fountain_aura`, a building's.
const FOUNTAIN_AURAS: [Aura; 1] = [Aura {
    whom: Whom::Allies,
    reaches: Reach::All,
    radius: rules::FOUNTAIN_HEAL_RADIUS,
    linger: rules::FOUNTAIN_HEAL_LINGER_TICKS,
    breakable: false,
    seen_only: false,
    water: false,
    modifier: AuraModifier::Kind(ModifierKind::Fountain {
        hp_pct: rules::FOUNTAIN_HEAL_HP_PCT,
        mana_pct: rules::FOUNTAIN_HEAL_MANA_PCT,
    }),
}];

pub const FOUNTAIN: UnitDef = UnitDef {
    kind: UnitKind::Fountain,
    max_hp: rules::FOUNTAIN_HP,
    auras: &FOUNTAIN_AURAS,
    damage: rules::FOUNTAIN_ATTACK_DAMAGE,
    attack_range: rules::FOUNTAIN_ATTACK_RANGE,
    acquisition: rules::FOUNTAIN_ATTACK_RANGE,
    attack_time: rules::FOUNTAIN_ATTACK_TIME,
    attack_point: rules::FOUNTAIN_ATTACK_POINT,
    attack_backswing: rules::FOUNTAIN_ATTACK_BACKSWING,
    projectile_speed: Some(rules::FOUNTAIN_PROJECTILE_SPEED),
    day_vision: rules::FOUNTAIN_VISION,
    night_vision: rules::FOUNTAIN_VISION,
    collision: rules::FOUNTAIN_COLLISION,
    bound: rules::FOUNTAIN_BOUND,
    invulnerable: true,
    ..BARE_UNIT
};

/// A courier: it carries, it does not fight, and it flies over everything.
pub const COURIER: UnitDef = UnitDef {
    kind: UnitKind::Courier,
    flies: true,
    porter: true,
    unbounded_pace: true,
    magic_resist_pct: 100,
    max_hp: rules::COURIER_HP,
    move_speed: rules::COURIER_MOVE_SPEED,
    turn_rate: rules::TURN_RATE_BRADS,
    day_vision: rules::COURIER_VISION,
    night_vision: rules::COURIER_VISION,
    ..BARE_UNIT
};

/// The slots a courier carries, in the order they are shown: Dota's `Ability1..` of its unit.
pub const COURIER_ABILITIES: [AbilityId; 5] = [
    AbilityId::TAKE_STASH,
    AbilityId::RETURN_ITEMS,
    AbilityId::BURST,
    AbilityId::DELIVER,
    AbilityId::SHIELD,
];

/// An observer ward: it sees far and the other side cannot see it.
pub const OBSERVER_WARD: UnitDef = UnitDef {
    kind: UnitKind::Ward,
    max_hp: rules::WARD_HP,
    day_vision: 1600,
    night_vision: 1600,
    hides: true,
    ..BARE_UNIT
};

/// A sentry ward: it sees nothing of itself, and what it gives is true sight.
pub const SENTRY_WARD: UnitDef = UnitDef {
    kind: UnitKind::Ward,
    max_hp: rules::WARD_HP,
    day_vision: 0,
    night_vision: 0,
    true_sight: 1050,
    hides: true,
    ..BARE_UNIT
};

/// What a tower of each tier keeps its own heroes in, indexed by tier less
/// one.
const TOWER_AURAS: [[Aura; 1]; 4] = [tower_aura(0), tower_aura(1), tower_aura(2), tower_aura(3)];

/// One tier's protection: Dota's `modifier_tower_aura`, a building's.
const fn tower_aura(index: usize) -> [Aura; 1] {
    [Aura {
        whom: Whom::Allies,
        reaches: Reach::Heroes,
        radius: rules::TOWER_AURA_RADIUS,
        linger: rules::AURA_LINGER_TICKS,
        breakable: false,
        seen_only: false,
        water: false,
        modifier: AuraModifier::Kind(ModifierKind::Guarded {
            armor: rules::TOWER_AURA_ARMOR[index],
            hp_per_second: rules::TOWER_AURA_REGEN[index],
        }),
    }]
}

/// A lane tower, by tier less one.
const fn tower_of(index: usize) -> UnitDef {
    UnitDef {
        kind: UnitKind::Tower,
        auras: &TOWER_AURAS[index],
        true_sight: rules::TOWER_ATTACK_RANGE,
        max_hp: rules::TOWER_TIER_HP[index],
        damage: rules::TOWER_TIER_DAMAGE[index],
        damage_spread: rules::TOWER_DAMAGE_SPREAD,
        attack_range: rules::TOWER_ATTACK_RANGE,
        acquisition: rules::TOWER_ATTACK_RANGE,
        attack_time: rules::TOWER_ATTACK_TIME,
        attack_speed: rules::TOWER_ATTACK_SPEED,
        attack_point: rules::TOWER_ATTACK_POINT,
        attack_backswing: rules::TOWER_ATTACK_BACKSWING,
        projectile_speed: Some(rules::TOWER_PROJECTILE_SPEED),
        armor: rules::TOWER_TIER_ARMOR[index],
        turn_rate: rules::TOWER_TURN_RATE_BRADS,
        day_vision: rules::TOWER_DAY_VISION,
        night_vision: rules::TOWER_NIGHT_VISION[index],
        collision: rules::TOWER_COLLISION,
        bound: rules::TOWER_BOUND,
        bounty_gold: rules::TOWER_TIER_BOUNTY[index],
        bounty_team: rules::TOWER_TIER_TEAM_BOUNTY[index],
        backdoor: match index {
            0 => None,
            1 => Some(BackdoorWatch::Own {
                radius: rules::BACKDOOR_TOWER_RADIUS,
            }),
            _ => Some(BackdoorWatch::InBase),
        },
        ..BARE_UNIT
    }
}

// The buildings' numbers are the game's `npc_units.txt` (client 6957,
// 2026-10-09) where it holds them; the bounties and the Ancient's collision
// are Liquipedia's *Tower (Tier 1)* to *(Tier 4)*, *Melee Barracks* and
// *Ancient* (read 2026-10-18).

/// Lane towers, indexed by tier less one.
pub const TOWERS: [UnitDef; 4] = [tower_of(0), tower_of(1), tower_of(2), tower_of(3)];

/// The plain form of a tower of this tier, counted from one.
#[inline]
pub fn tower_def(tier: u8) -> &'static UnitDef {
    &TOWERS[usize::from(tier.clamp(1, 4)) - 1]
}

/// The definition of a lane creep kind. Absent for every other kind.
#[inline]
pub const fn lane_creep_def(kind: UnitKind) -> Option<&'static UnitDef> {
    match kind {
        UnitKind::CreepMelee => Some(&MELEE_CREEP),
        UnitKind::CreepFlagbearer => Some(&FLAGBEARER_CREEP),
        UnitKind::CreepRanged => Some(&RANGED_CREEP),
        UnitKind::CreepSiege => Some(&SIEGE_CREEP),
        _ => None,
    }
}
