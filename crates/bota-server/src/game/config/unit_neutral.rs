//! Every kind of neutral creep, straight out of the shipped unit tables.
//!
//! Generated. The camp a kind belongs to and what a camp draws are elsewhere;
//! this is only what one of them is worth.

use bota_proto::{Attributes, Fixed, UnitKind};

use crate::game::rules;
use crate::game::{Growth, UnitDef};

/// One kind of neutral creep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NeutralKind {
    /// `npc_dota_neutral_kobold`.
    Kobold,
    /// `npc_dota_neutral_kobold_tunneler`.
    KoboldTunneler,
    /// `npc_dota_neutral_kobold_taskmaster`.
    KoboldTaskmaster,
    /// `npc_dota_neutral_forest_troll_berserker`.
    ForestTrollBerserker,
    /// `npc_dota_neutral_forest_troll_high_priest`.
    ForestTrollHighPriest,
    /// `npc_dota_neutral_gnoll_assassin`.
    GnollAssassin,
    /// `npc_dota_neutral_fel_beast`.
    FelBeast,
    /// `npc_dota_neutral_ghost`.
    Ghost,
    /// `npc_dota_neutral_harpy_scout`.
    HarpyScout,
    /// `npc_dota_neutral_harpy_storm`.
    HarpyStorm,
    /// `npc_dota_neutral_centaur_outrunner`.
    CentaurOutrunner,
    /// `npc_dota_neutral_centaur_khan`.
    CentaurKhan,
    /// `npc_dota_neutral_giant_wolf`.
    GiantWolf,
    /// `npc_dota_neutral_alpha_wolf`.
    AlphaWolf,
    /// `npc_dota_neutral_satyr_trickster`.
    SatyrTrickster,
    /// `npc_dota_neutral_satyr_soulstealer`.
    SatyrSoulstealer,
    /// `npc_dota_neutral_satyr_hellcaller`.
    SatyrHellcaller,
    /// `npc_dota_neutral_ogre_mauler`.
    OgreMauler,
    /// `npc_dota_neutral_ogre_magi`.
    OgreMagi,
    /// `npc_dota_neutral_mud_golem`.
    MudGolem,
    /// `npc_dota_neutral_mud_golem_split`.
    MudGolemSplit,
    /// `npc_dota_neutral_polar_furbolg_champion`.
    PolarFurbolgChampion,
    /// `npc_dota_neutral_polar_furbolg_ursa_warrior`.
    PolarFurbolgUrsaWarrior,
    /// `npc_dota_neutral_wildkin`.
    Wildkin,
    /// `npc_dota_neutral_enraged_wildkin`.
    EnragedWildkin,
    /// `npc_dota_neutral_dark_troll`.
    DarkTroll,
    /// `npc_dota_neutral_dark_troll_warlord`.
    DarkTrollWarlord,
    /// `npc_dota_neutral_warpine_raider`.
    WarpineRaider,
    /// `npc_dota_neutral_black_drake`.
    BlackDrake,
    /// `npc_dota_neutral_black_dragon`.
    BlackDragon,
    /// `npc_dota_neutral_rock_golem`.
    RockGolem,
    /// `npc_dota_neutral_granite_golem`.
    GraniteGolem,
    /// `npc_dota_neutral_small_thunder_lizard`.
    SmallThunderLizard,
    /// `npc_dota_neutral_big_thunder_lizard`.
    BigThunderLizard,
    /// `npc_dota_neutral_frostbitten_golem`.
    FrostbittenGolem,
    /// `npc_dota_neutral_ice_shaman`.
    IceShaman,
}

impl NeutralKind {
    /// The plain form this kind spawns with.
    pub fn def(self) -> &'static UnitDef {
        &NEUTRALS[self as usize]
    }
}

/// What one upgrade interval adds to any neutral.
const NEUTRAL_GROWTH: Growth = Growth {
    attributes: Attributes::ZERO,
    hp: rules::NEUTRAL_UPGRADE_HP,
    mana: 0,
    damage: rules::NEUTRAL_UPGRADE_DAMAGE,
    armor_halves: rules::NEUTRAL_UPGRADE_ARMOR_HALVES,
    gold: rules::NEUTRAL_UPGRADE_GOLD,
    xp: rules::NEUTRAL_UPGRADE_XP,
};

/// What every neutral shares, so a kind names only its own numbers.
const NEUTRAL_BASE: UnitDef = UnitDef {
    kind: UnitKind::CreepNeutral,
    attributes: Attributes::ZERO,
    primary: None,
    max_hp: 0,
    max_mana: 0,
    hp_regen: Fixed::ZERO,
    mana_regen: Fixed::ZERO,
    damage: 0,
    attack_range: 0,
    acquisition: 0,
    attack_time: 0,
    attack_point: 0,
    attack_backswing: 0,
    projectile_speed: None,
    armor: 0,
    magic_resist_pct: 0,
    move_speed: 0,
    turn_rate: 0,
    vision: rules::NEUTRAL_VISION,
    radius: rules::NEUTRAL_RADIUS,
    invulnerable: false,
    ancient: false,
    bounty_gold: 0,
    bounty_xp: 0,
    per_level: Growth {
        attributes: Attributes::ZERO,
        hp: 0,
        mana: 0,
        damage: 0,
        armor_halves: 0,
        gold: 0,
        xp: 0,
    },
    per_upgrade: NEUTRAL_GROWTH,
    auras: &[],
    true_sight: 0,
    hides: false,
    flies: false,
    porter: false,
};

/// Every kind, in the order of [`NeutralKind`].
pub const NEUTRALS: [UnitDef; 36] = [
    // Kobold
    UnitDef {
        max_hp: 240,
        damage: 15,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 366,
        projectile_speed: None,
        armor: 0,
        magic_resist_pct: 0,
        move_speed: 290,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 4,
        bounty_xp: 14,
        ..NEUTRAL_BASE
    },
    // KoboldTunneler
    UnitDef {
        max_hp: 325,
        damage: 22,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 366,
        projectile_speed: None,
        armor: 1,
        magic_resist_pct: 0,
        move_speed: 270,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 13,
        bounty_xp: 17,
        ..NEUTRAL_BASE
    },
    // KoboldTaskmaster
    UnitDef {
        max_hp: 400,
        damage: 25,
        attack_range: 110,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 366,
        projectile_speed: None,
        armor: 2,
        magic_resist_pct: 0,
        move_speed: 330,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 20,
        bounty_xp: 30,
        ..NEUTRAL_BASE
    },
    // ForestTrollBerserker
    UnitDef {
        max_hp: 500,
        damage: 32,
        attack_range: 500,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: Some(1200),
        armor: 1,
        magic_resist_pct: 0,
        move_speed: 270,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 19,
        bounty_xp: 28,
        ..NEUTRAL_BASE
    },
    // ForestTrollHighPriest
    UnitDef {
        max_hp: 450,
        damage: 31,
        attack_range: 600,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: Some(900),
        armor: 0,
        magic_resist_pct: 0,
        move_speed: 290,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 19,
        bounty_xp: 28,
        ..NEUTRAL_BASE
    },
    // GnollAssassin
    UnitDef {
        max_hp: 400,
        damage: 26,
        attack_range: 500,
        acquisition: 800,
        attack_time: 2000,
        attack_point: 400,
        projectile_speed: Some(1500),
        armor: 1,
        magic_resist_pct: 0,
        move_speed: 270,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 17,
        bounty_xp: 30,
        ..NEUTRAL_BASE
    },
    // FelBeast
    UnitDef {
        max_hp: 400,
        damage: 14,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 400,
        projectile_speed: None,
        armor: 1,
        magic_resist_pct: 0,
        move_speed: 350,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 17,
        bounty_xp: 26,
        ..NEUTRAL_BASE
    },
    // Ghost
    UnitDef {
        max_hp: 500,
        damage: 40,
        attack_range: 400,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: Some(900),
        armor: 2,
        magic_resist_pct: 0,
        move_speed: 320,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 24,
        bounty_xp: 42,
        ..NEUTRAL_BASE
    },
    // HarpyScout
    UnitDef {
        max_hp: 400,
        damage: 31,
        attack_range: 300,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: Some(1200),
        armor: 1,
        magic_resist_pct: 0,
        move_speed: 280,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 15,
        bounty_xp: 26,
        ..NEUTRAL_BASE
    },
    // HarpyStorm
    UnitDef {
        max_hp: 500,
        damage: 33,
        attack_range: 450,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: Some(1200),
        armor: 2,
        magic_resist_pct: 0,
        move_speed: 310,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 26,
        bounty_xp: 42,
        ..NEUTRAL_BASE
    },
    // CentaurOutrunner
    UnitDef {
        max_hp: 350,
        damage: 19,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 1,
        magic_resist_pct: 0,
        move_speed: 320,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 17,
        bounty_xp: 32,
        ..NEUTRAL_BASE
    },
    // CentaurKhan
    UnitDef {
        max_hp: 1100,
        damage: 52,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 4,
        magic_resist_pct: 0,
        move_speed: 320,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 57,
        bounty_xp: 90,
        ..NEUTRAL_BASE
    },
    // GiantWolf
    UnitDef {
        max_hp: 500,
        damage: 16,
        attack_range: 90,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 333,
        projectile_speed: None,
        armor: 1,
        magic_resist_pct: 0,
        move_speed: 350,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 20,
        bounty_xp: 40,
        ..NEUTRAL_BASE
    },
    // AlphaWolf
    UnitDef {
        max_hp: 600,
        damage: 28,
        attack_range: 90,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 333,
        projectile_speed: None,
        armor: 3,
        magic_resist_pct: 0,
        move_speed: 350,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 33,
        bounty_xp: 60,
        ..NEUTRAL_BASE
    },
    // SatyrTrickster
    UnitDef {
        max_hp: 300,
        damage: 11,
        attack_range: 280,
        acquisition: 280,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: Some(1500),
        armor: 0,
        magic_resist_pct: 0,
        move_speed: 300,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 13,
        bounty_xp: 24,
        ..NEUTRAL_BASE
    },
    // SatyrSoulstealer
    UnitDef {
        max_hp: 600,
        damage: 22,
        attack_range: 100,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 2,
        magic_resist_pct: 0,
        move_speed: 270,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 20,
        bounty_xp: 46,
        ..NEUTRAL_BASE
    },
    // SatyrHellcaller
    UnitDef {
        max_hp: 1100,
        damage: 52,
        attack_range: 100,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 2,
        magic_resist_pct: 0,
        move_speed: 290,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 63,
        bounty_xp: 90,
        ..NEUTRAL_BASE
    },
    // OgreMauler
    UnitDef {
        max_hp: 800,
        damage: 23,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 1,
        magic_resist_pct: 0,
        move_speed: 270,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 24,
        bounty_xp: 32,
        ..NEUTRAL_BASE
    },
    // OgreMagi
    UnitDef {
        max_hp: 600,
        damage: 19,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 0,
        magic_resist_pct: 0,
        move_speed: 270,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 30,
        bounty_xp: 48,
        ..NEUTRAL_BASE
    },
    // MudGolem
    UnitDef {
        max_hp: 750,
        damage: 25,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 0,
        magic_resist_pct: 30,
        move_speed: 310,
        turn_rate: 5795,
        ancient: false,
        bounty_gold: 20,
        bounty_xp: 32,
        ..NEUTRAL_BASE
    },
    // MudGolemSplit
    UnitDef {
        max_hp: 250,
        damage: 12,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 0,
        magic_resist_pct: 33,
        move_speed: 310,
        turn_rate: 5795,
        ancient: false,
        bounty_gold: 8,
        bounty_xp: 18,
        ..NEUTRAL_BASE
    },
    // PolarFurbolgChampion
    UnitDef {
        max_hp: 700,
        damage: 41,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 3,
        magic_resist_pct: 0,
        move_speed: 320,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 34,
        bounty_xp: 66,
        ..NEUTRAL_BASE
    },
    // PolarFurbolgUrsaWarrior
    UnitDef {
        max_hp: 950,
        damage: 52,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 4,
        magic_resist_pct: 0,
        move_speed: 320,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 64,
        bounty_xp: 90,
        ..NEUTRAL_BASE
    },
    // Wildkin
    UnitDef {
        max_hp: 350,
        damage: 19,
        attack_range: 128,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 2,
        magic_resist_pct: 0,
        move_speed: 300,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 17,
        bounty_xp: 26,
        ..NEUTRAL_BASE
    },
    // EnragedWildkin
    UnitDef {
        max_hp: 950,
        damage: 53,
        attack_range: 128,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 4,
        magic_resist_pct: 0,
        move_speed: 320,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 61,
        bounty_xp: 90,
        ..NEUTRAL_BASE
    },
    // DarkTroll
    UnitDef {
        max_hp: 500,
        damage: 25,
        attack_range: 250,
        acquisition: 250,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: Some(1200),
        armor: 0,
        magic_resist_pct: 0,
        move_speed: 270,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 18,
        bounty_xp: 42,
        ..NEUTRAL_BASE
    },
    // DarkTrollWarlord
    UnitDef {
        max_hp: 1100,
        damage: 42,
        attack_range: 250,
        acquisition: 250,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: Some(1200),
        armor: 4,
        magic_resist_pct: 0,
        move_speed: 300,
        turn_rate: 10431,
        ancient: false,
        bounty_gold: 43,
        bounty_xp: 90,
        ..NEUTRAL_BASE
    },
    // WarpineRaider
    UnitDef {
        max_hp: 850,
        damage: 40,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 6,
        magic_resist_pct: 30,
        move_speed: 310,
        turn_rate: 5795,
        ancient: false,
        bounty_gold: 49,
        bounty_xp: 76,
        ..NEUTRAL_BASE
    },
    // BlackDrake
    UnitDef {
        max_hp: 950,
        damage: 21,
        attack_range: 300,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 500,
        projectile_speed: Some(900),
        armor: 2,
        magic_resist_pct: 25,
        move_speed: 350,
        turn_rate: 10431,
        ancient: true,
        bounty_gold: 40,
        bounty_xp: 95,
        ..NEUTRAL_BASE
    },
    // BlackDragon
    UnitDef {
        max_hp: 2000,
        damage: 65,
        attack_range: 300,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 500,
        projectile_speed: Some(1500),
        armor: 4,
        magic_resist_pct: 30,
        move_speed: 300,
        turn_rate: 10431,
        ancient: true,
        bounty_gold: 78,
        bounty_xp: 124,
        ..NEUTRAL_BASE
    },
    // RockGolem
    UnitDef {
        max_hp: 800,
        damage: 23,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 4,
        magic_resist_pct: 30,
        move_speed: 270,
        turn_rate: 5795,
        ancient: true,
        bounty_gold: 40,
        bounty_xp: 95,
        ..NEUTRAL_BASE
    },
    // GraniteGolem
    UnitDef {
        max_hp: 1500,
        damage: 82,
        attack_range: 128,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 8,
        magic_resist_pct: 30,
        move_speed: 270,
        turn_rate: 5795,
        ancient: true,
        bounty_gold: 78,
        bounty_xp: 124,
        ..NEUTRAL_BASE
    },
    // SmallThunderLizard
    UnitDef {
        max_hp: 800,
        damage: 33,
        attack_range: 300,
        acquisition: 800,
        attack_time: 1800,
        attack_point: 500,
        projectile_speed: Some(1500),
        armor: 3,
        magic_resist_pct: 50,
        move_speed: 270,
        turn_rate: 5795,
        ancient: true,
        bounty_gold: 45,
        bounty_xp: 95,
        ..NEUTRAL_BASE
    },
    // BigThunderLizard
    UnitDef {
        max_hp: 1700,
        damage: 62,
        attack_range: 300,
        acquisition: 300,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: Some(1500),
        armor: 3,
        magic_resist_pct: 30,
        move_speed: 270,
        turn_rate: 10431,
        ancient: true,
        bounty_gold: 78,
        bounty_xp: 124,
        ..NEUTRAL_BASE
    },
    // FrostbittenGolem
    UnitDef {
        max_hp: 900,
        damage: 30,
        attack_range: 100,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 300,
        projectile_speed: None,
        armor: 7,
        magic_resist_pct: 30,
        move_speed: 300,
        turn_rate: 5795,
        ancient: true,
        bounty_gold: 40,
        bounty_xp: 95,
        ..NEUTRAL_BASE
    },
    // IceShaman
    UnitDef {
        max_hp: 1500,
        damage: 60,
        attack_range: 500,
        acquisition: 500,
        attack_time: 2000,
        attack_point: 700,
        projectile_speed: Some(1500),
        armor: 3,
        magic_resist_pct: 30,
        move_speed: 290,
        turn_rate: 10431,
        ancient: true,
        bounty_gold: 78,
        bounty_xp: 124,
        ..NEUTRAL_BASE
    },
];
