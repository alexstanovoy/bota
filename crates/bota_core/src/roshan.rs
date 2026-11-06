//! Roshan (`npc_dota_roshan`) of Dota 7.41f: the boss of the river pits, a
//! neutral with its own row, walking between his two pits as day and night
//! turn from 15:00, and leaving the Aegis and more where he falls.
//!
//! Sources, tagged where each number is used: `[U:n]` and `[A:n]` the lines
//! of the game's `scripts/npc/npc_units.txt` and `npc_abilities.txt` (client
//! 6972, 7.41f); `[I:n]` its `scripts/npc/items.txt`; `[ENT:n]` the entity
//! blocks of the map's `maps/dota/entities/default_ents.vents`, shifted by
//! half the map like every landmark; `[BIN]` a server convar default of the
//! 6966 build; `[PN v]` Valve's patch notes of version `v`; `[LIQ page]`
//! Liquipedia's article (read 2026-11-06).

use bota_proto::{Fixed, ObjectiveKind, UnitKind, Vec2};

use crate::data::ROSHAN_UNIT as U;
use crate::unit::BARE_UNIT;
use crate::{GoldRange, HomeDef, LairDef, NeutralKind, UnitDef, rules};

/// His numbers [U:12091-12177]: 6000 health and 20 regeneration, no mana,
/// armor 30 and magic resistance 55, a melee attack of 75 every 2.0 s with
/// its point at 0.6 s (0.3 s after it [LIQ Roshan]) and 150 reach, moving at
/// 270 and turning at 1.0, seeing 1400 by day and night, a hero's hull, and
/// a fall worth 200-290 gold and 400 experience to his killer, and 135 gold
/// to every hero of the killing side [LIQ Roshan]. Attacks reach him only
/// from inside a pit [LIQ Roshan], and he is ancient [U:12100].
pub const ROSHAN: UnitDef = UnitDef {
    kind: UnitKind::Roshan,
    base_level: U.first("Level").int() as u8,
    max_hp: U.first("StatusHealth").int(),
    hp_regen: Fixed::from_ratio(
        U.first("StatusHealthRegen").int(),
        rules::TICKS_PER_SECOND as i32,
    ),
    damage: U.first("AttackDamageMin").int(),
    damage_spread: U.first("AttackDamageMax").int() - U.first("AttackDamageMin").int(),
    attack_range: U.first("AttackRange").int(),
    acquisition: U.first("AttackAcquisitionRange").int(),
    attack_time: U.first("AttackRate").0 as u32,
    attack_speed: 200, // PN 6.86: BAT 2.0 and +100 attack speed; absent from npc_units.txt.
    attack_point: U.first("AttackAnimationPoint").0 as u32,
    attack_backswing: 300,
    projectile_speed: None,
    armor: U.first("ArmorPhysical").int(),
    magic_resist_pct: U.first("MagicalResistance").int(),
    move_speed: U.first("MovementSpeed").int(),
    turn_rate: rules::FAST_TURN_RATE_BRADS,
    day_vision: U.first("VisionDaytimeRange").int(),
    night_vision: U.first("VisionNighttimeRange").int(),
    collision: rules::HERO_COLLISION,
    bound: rules::HERO_BOUND,
    ancient: U.first("IsAncient").int() != 0,
    bounty_gold: GoldRange::new(
        U.first("BountyGoldMin").int(),
        U.first("BountyGoldMax").int(),
    ),
    bounty_team: 135,
    bounty_xp: U.first("BountyXP").int(),
    pit_attacked: true,
    per_upgrade: crate::Growth {
        hp: 130,
        damage: 7,
        xp: 20,
        ..crate::unit::NO_GROWTH
    },
    innate: &[crate::ModifierKind::Phased],
    ..BARE_UNIT
};

/// How he holds his pit: woken by an enemy within 140 or a blow from within
/// 1800, never asleep, led off it as a neutral is [LIQ Roshan "Behavior";
/// LIQ Neutral Creeps].
pub const ROSHAN_HOME: HomeDef = HomeDef {
    attacks_couriers: true,
    aggro_range: 140,
    damage_aggro_range: 1800,
    aggro_spread: 0,
    sleeps: false,
    ..crate::JUNGLE_HOME
};

/// The Dire pit, top-left of the river, where he stands by day and until
/// 15:00 [LIQ Roshan "On The Move"; PN 7.41 "pit preference has switched"]:
/// `info_player_start_dota` "roshan_location_2" (-3194.28, 2394.62)
/// [ENT:186935].
pub const ROSHAN_DAY_PIT: Vec2 = Vec2::from_ints(6022, 11611);
/// The Radiant pit, bottom-right of the river, where he stands by night:
/// "roshan_location_1" (2859.76, -2764.77) [ENT:187040].
pub const ROSHAN_NIGHT_PIT: Vec2 = Vec2::from_ints(12076, 6451);

/// Present from match setup; walking begins at 15:00. 7.41 switched pit preferences.
pub const ROSHAN_LAIR: LairDef = LairDef {
    objective: ObjectiveKind::Roshan,
    neutral: NeutralKind::Roshan,
    day: ROSHAN_DAY_PIT,
    night: ROSHAN_NIGHT_PIT,
    move_after: 15 * 60,
    respawn: [8 * 60, 11 * 60],
    drops: &ROSHAN_DROPS,
};

const _: () = assert!(U.first("MovementTurnRate").int() == 1);

/// 7.37 made Banner permanent from the second death; 7.38 removed Blessing.
/// Current 7.41f drop table is independent of day and night (Roshan, Kill Rewards).
pub const ROSHAN_DROPS: [crate::LairDrop; 4] = [
    crate::LairDrop {
        item: bota_proto::ItemId::AEGIS,
        from: 1,
        reclaimed: true,
        max_level: 1,
    },
    crate::LairDrop {
        item: bota_proto::ItemId::ROSHANS_BANNER,
        from: 2,
        reclaimed: false,
        max_level: 3,
    },
    crate::LairDrop {
        item: bota_proto::ItemId::CHEESE,
        from: 3,
        reclaimed: false,
        max_level: 1,
    },
    crate::LairDrop {
        item: bota_proto::ItemId::REFRESHER_SHARD,
        from: 3,
        reclaimed: false,
        max_level: 1,
    },
];

/// Devotion carries only the armor to nearby allies.
pub(crate) const ROSHAN_ARMOR_AURA: crate::Aura = crate::Aura {
    whom: crate::Whom::Allies,
    reaches: crate::Reach::Living,
    radius: crate::data::ROSHAN_DEVOTION.first("radius").int(),
    linger: rules::TICKS_PER_SECOND / 2,
    breakable: false,
    invulnerable: false,
    water: false,
    growth_clock: Some(crate::GrowthClock {
        first_after_horn: 0,
        period: 60 * rules::TICKS_PER_SECOND,
        cap: u32::MAX,
    }),
    modifier: crate::AuraModifier::ArmorGrowth {
        hundredths_per_step: 40,
        effect: bota_proto::EffectId(79),
    },
};
