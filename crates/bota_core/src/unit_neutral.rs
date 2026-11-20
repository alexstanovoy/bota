//! Every kind of neutral creep: one entry each, its numbers and what it
//! can do.
//!
//! The numbers are the game's own, read from `scripts/npc/npc_units.txt` by `tools/import_items`
//! into [`neutral_data`](crate::neutral_data) (health and its regeneration, mana and its
//! regeneration, damage range, attack range, acquisition range, base attack time, base attack
//! speed, attack point, missile speed, armor, magic resistance, move speed, turn rate, day and
//! night vision, whether it is ancient, bounty) with the abilities each carries; what that file
//! does not hold is kept, with its source, in the importer's `neutral_overrides.rs`. The hull is
//! the game's: a unit that names none has the hero's, as the template they inherit does. What an
//! ability does is [`NeutralAbility`]'s; the camp a kind belongs to and what a camp draws are
//! [`ROSTERS`](crate::ROSTERS)'.
//!
//! The npc files carry no backswing, so `attack_backswing` is Liquipedia's *Attack Animation*
//! table (see the importer's overrides). `Level` is not read: a neutral's `base_level` stays 1.
//!
//! A new kind that needs no new mechanic is one variant appended to
//! [`NeutralKind`] (its id on the wire, never renumbered), one entry appended
//! to [`NEUTRALS`], and its line in the protocol's id registry, then a run of the importer.

use bota_proto::{Fixed, NeutralId, UnitKind};

use crate::neutral_data as script;
use crate::table::placed_by_id;
use crate::unit::{BARE_UNIT, NO_GROWTH};
use crate::{Dec, GoldRange, Growth, NeutralAbility, ScriptData, UnitDef, rules, turn_rate_brads};

use NeutralAbility as A;

/// One kind of neutral creep, by the game's unit name. Its place is its
/// [`NeutralId`]: append-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NeutralKind {
    /// Kobold: `npc_dota_neutral_kobold`.
    Kobold,
    /// Kobold Soldier: `npc_dota_neutral_kobold_tunneler`.
    KoboldTunneler,
    /// Kobold Foreman: `npc_dota_neutral_kobold_taskmaster`.
    KoboldTaskmaster,
    /// Hill Troll Berserker: `npc_dota_neutral_forest_troll_berserker`.
    ForestTrollBerserker,
    /// Hill Troll Priest: `npc_dota_neutral_forest_troll_high_priest`.
    ForestTrollHighPriest,
    /// Vhoul Assassin: `npc_dota_neutral_gnoll_assassin`.
    GnollAssassin,
    /// Fell Spirit: `npc_dota_neutral_fel_beast`.
    FelBeast,
    /// Ghost: `npc_dota_neutral_ghost`.
    Ghost,
    /// Harpy Scout: `npc_dota_neutral_harpy_scout`.
    HarpyScout,
    /// Harpy Stormcrafter: `npc_dota_neutral_harpy_storm`.
    HarpyStorm,
    /// Centaur Courser: `npc_dota_neutral_centaur_outrunner`.
    CentaurOutrunner,
    /// Centaur Conqueror: `npc_dota_neutral_centaur_khan`.
    CentaurKhan,
    /// Giant Wolf: `npc_dota_neutral_giant_wolf`.
    GiantWolf,
    /// Alpha Wolf: `npc_dota_neutral_alpha_wolf`.
    AlphaWolf,
    /// Satyr Banisher: `npc_dota_neutral_satyr_trickster`.
    SatyrTrickster,
    /// Satyr Mindstealer: `npc_dota_neutral_satyr_soulstealer`.
    SatyrSoulstealer,
    /// Satyr Tormenter: `npc_dota_neutral_satyr_hellcaller`.
    SatyrHellcaller,
    /// Ogre Bruiser: `npc_dota_neutral_ogre_mauler`.
    OgreMauler,
    /// Ogre Frostmage: `npc_dota_neutral_ogre_magi`.
    OgreMagi,
    /// Mud Golem: `npc_dota_neutral_mud_golem`.
    MudGolem,
    /// Shard Golem: `npc_dota_neutral_mud_golem_split`.
    MudGolemSplit,
    /// Hellbear: `npc_dota_neutral_polar_furbolg_champion`.
    PolarFurbolgChampion,
    /// Hellbear Smasher: `npc_dota_neutral_polar_furbolg_ursa_warrior`.
    PolarFurbolgUrsaWarrior,
    /// Wildwing: `npc_dota_neutral_wildkin`.
    Wildkin,
    /// Wildwing Ripper: `npc_dota_neutral_enraged_wildkin`.
    EnragedWildkin,
    /// Hill Troll: `npc_dota_neutral_dark_troll`.
    DarkTroll,
    /// Dark Troll Summoner: `npc_dota_neutral_dark_troll_warlord`.
    DarkTrollWarlord,
    /// Warpine Raider: `npc_dota_neutral_warpine_raider`.
    WarpineRaider,
    /// Ancient Black Drake: `npc_dota_neutral_black_drake`.
    BlackDrake,
    /// Ancient Black Dragon: `npc_dota_neutral_black_dragon`.
    BlackDragon,
    /// Ancient Rock Golem: `npc_dota_neutral_rock_golem`.
    RockGolem,
    /// Ancient Granite Golem: `npc_dota_neutral_granite_golem`.
    GraniteGolem,
    /// Ancient Rumblehide: `npc_dota_neutral_small_thunder_lizard`.
    SmallThunderLizard,
    /// Ancient Thunderhide: `npc_dota_neutral_big_thunder_lizard`.
    BigThunderLizard,
    /// Ancient Frostbitten Golem: `npc_dota_neutral_frostbitten_golem`.
    FrostbittenGolem,
    /// Ancient Ice Shaman: `npc_dota_neutral_ice_shaman`.
    IceShaman,
    /// Ancient Prowler Acolyte: `npc_dota_neutral_prowler_acolyte`.
    ProwlerAcolyte,
    /// Ancient Prowler Shaman: `npc_dota_neutral_prowler_shaman`.
    ProwlerShaman,
    /// Pollywog: `npc_dota_neutral_tadpole`.
    Tadpole,
    /// Boglet: `npc_dota_neutral_froglet`.
    Froglet,
    /// Croaker: `npc_dota_neutral_grown_frog`.
    GrownFrog,
    /// Ancient Croaker: `npc_dota_neutral_ancient_frog`.
    AncientFrog,
    /// Marshmage Apprentice: `npc_dota_neutral_froglet_mage`.
    FrogletMage,
    /// Marshmage: `npc_dota_neutral_grown_frog_mage`.
    GrownFrogMage,
    /// Ancient Marshmage: `npc_dota_neutral_ancient_frog_mage`.
    AncientFrogMage,
    /// Skeleton Warrior: `npc_dota_dark_troll_warlord_skeleton_warrior`.
    SkeletonWarrior,
    /// Tornado: `npc_dota_enraged_wildkin_tornado`.
    WildkinTornado,
    /// Boss of the river pits: `npc_dota_roshan`.
    Roshan,
    /// Radigull: `npc_dota_neutral_radigull` (Dota 7.42).
    Radigull,
    /// Odobenus Footman: `npc_dota_neutral_odobenus_footman` (Dota 7.42).
    OdobenusFootman,
    /// Odobenus Frostmage: `npc_dota_neutral_odobenus_frostmage` (Dota 7.42).
    OdobenusFrostmage,
    /// Eimermole: `npc_dota_neutral_eimermole` (Dota 7.42).
    Eimermole,
    /// Eimermole Foreman: `npc_dota_neutral_eimermole_foreman` (Dota 7.42).
    EimermoleForeman,
    /// Ancient Ogre Seal: `npc_dota_neutral_ogre_seal` (Dota 7.42).
    OgreSeal,
    /// Ancient Ogre Seal Pup: `npc_dota_neutral_ogre_seal_pup` (Dota 7.42).
    OgreSealPup,
    /// The golden Kobold Soldier: `npc_dota_neutral_kobold_tunneler_gold` (Dota 7.42).
    KoboldTunnelerGold,
    /// The golden Vhoul Assassin: `npc_dota_neutral_gnoll_assassin_gold` (Dota 7.42).
    GnollAssassinGold,
    /// The golden Alpha Wolf: `npc_dota_neutral_alpha_wolf_gold` (Dota 7.42).
    AlphaWolfGold,
    /// The golden Mud Golem: `npc_dota_neutral_mud_golem_gold` (Dota 7.42).
    MudGolemGold,
    /// The golden Wildwing Ripper: `npc_dota_neutral_enraged_wildkin_gold` (Dota 7.42).
    EnragedWildkinGold,
    /// The golden Hellbear: `npc_dota_neutral_polar_furbolg_champion_gold` (Dota 7.42).
    PolarFurbolgChampionGold,
}

impl NeutralKind {
    /// The kind of a wire id, if there is one.
    pub fn of(id: NeutralId) -> Option<NeutralKind> {
        NEUTRALS.get(usize::from(id.0)).map(|neutral| neutral.kind)
    }

    /// Its id on the wire.
    pub const fn id(self) -> NeutralId {
        NeutralId(self as u16)
    }

    /// Its entry.
    pub const fn neutral(self) -> &'static NeutralDef {
        &NEUTRALS[self as usize]
    }

    /// The plain form this kind spawns with.
    pub const fn def(self) -> &'static UnitDef {
        &self.neutral().def
    }
}

/// One kind of neutral creep: what it is called, its numbers, and its
/// abilities in slot order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralDef {
    /// The kind the entry is for: its place in [`NEUTRALS`].
    pub kind: NeutralKind,
    /// Dota's internal name of the unit, its line of the id registry: what
    /// its model, its art and every data file know it by.
    pub unit: &'static str,
    /// What it can do, in slot order.
    pub abilities: &'static [NeutralAbility],
    /// Its permanent self growth: the source ability and the clock that raises
    /// stats and bounty. Absent for a neutral that does not grow.
    pub growth: Option<crate::NeutralGrowth>,
    /// Whether camps may put it out: false for a kind the registry retired.
    pub offered: bool,
    /// The plain form it spawns with.
    pub def: UnitDef,
    /// Its wake and leash policy.
    pub home: &'static HomeDef,
}

/// How a neutral holds the ground it stands guard on, as Liquipedia's
/// *Neutral Creeps* ("Behavior") gives it: what draws it, how far it is led
/// before it gives up and walks back, and whether it sleeps at night.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HomeDef {
    /// Whether this autonomous neutral may acquire couriers.
    pub attacks_couriers: bool,
    /// Prefer designated wards in attack reach over chasing other targets.
    pub wards_first: bool,
    /// A hostile unit this close wakes it.
    pub aggro_range: i32,
    /// A blow struck from within this range wakes it.
    pub damage_aggro_range: i32,
    /// How far around it a blow on it wakes the others of a camp.
    pub aggro_spread: i32,
    /// How far from its spot it may stand before its aggro window starts
    /// running down: the guard distance.
    pub guard: i32,
    /// Ticks it may spend beyond its guard distance before it walks home.
    pub window: u32,
    /// The window of one struck back from its walk home.
    pub short_window: u32,
    /// Ticks after a leash break in which nothing wakes it.
    pub reaggro_block: u32,
    /// Being this close to its spot ends its walk home.
    pub back: i32,
    /// Whether it sleeps by night: nothing coming near wakes it then
    /// (Liquipedia, *Time of Day*: a neutral's aggro range is 0 by night).
    pub sleeps: bool,
}

/// How the jungle's creeps hold their camps.
pub const JUNGLE_HOME: HomeDef = HomeDef {
    attacks_couriers: false,
    wards_first: false,
    aggro_range: rules::NEUTRAL_AGGRO_RANGE,
    damage_aggro_range: rules::NEUTRAL_DAMAGE_AGGRO_RANGE,
    aggro_spread: rules::NEUTRAL_AGGRO_SPREAD,
    guard: rules::NEUTRAL_GUARD_DISTANCE,
    window: rules::NEUTRAL_AGGRO_WINDOW,
    short_window: rules::NEUTRAL_SHORT_WINDOW,
    reaggro_block: rules::NEUTRAL_REAGGRO_BLOCK,
    back: rules::NEUTRAL_RETURN,
    sleeps: true,
};

/// The most abilities a neutral carries.
pub const MAX_NEUTRAL_ABILITIES: usize = 7;

const _: () = {
    let mut at = 0;
    while at < NEUTRALS.len() {
        let neutral = &NEUTRALS[at];
        assert!(neutral.abilities.len() <= MAX_NEUTRAL_ABILITIES);
        let mut slot = 0;
        let mut hidden = false;
        while slot < neutral.abilities.len() {
            let def = &neutral.abilities[slot].entry().def;
            assert!(
                !hidden || def.hidden,
                "visible neutral ability after hidden slot"
            );
            hidden |= def.hidden;
            slot += 1;
        }
        assert!(crate::kind_def(neutral.def.kind).neutral);
        at += 1;
    }
};

// The kinds are the script rows, in the registry's order (Roshan, whose rows are his own, aside),
// and each carries the abilities its script lists in slot order (`neutral_upgrade` is its growth,
// not a slot of ours): a patch that changes a unit's slots fails the build, naming nothing to
// guess at.
const _: () = {
    use crate::item_data::str_eq;
    assert!(
        script::UNIT_ROWS.len() + 1 == NEUTRALS.len(),
        "the script rows and the kinds differ in number"
    );
    let mut at = 0;
    let mut row = 0;
    while at < NEUTRALS.len() {
        let neutral = &NEUTRALS[at];
        at += 1;
        if neutral.kind as u8 == NeutralKind::Roshan as u8 {
            continue;
        }
        let data = script::UNIT_ROWS[row];
        row += 1;
        assert!(
            str_eq(neutral.unit, data.key),
            "the neutral kinds are not the script rows in the registry's order"
        );
        let mut slot = 0;
        let mut carried = 0;
        while slot < data.abilities.len() {
            if !str_eq(data.abilities[slot], "neutral_upgrade") {
                assert!(
                    carried < neutral.abilities.len(),
                    "a slot of a neutral's script is no ability of its kind"
                );
                assert!(
                    str_eq(
                        neutral.abilities[carried].entry().def.key,
                        data.abilities[slot]
                    ),
                    "a neutral kind's abilities are not its script's slots in order"
                );
                carried += 1;
            }
            slot += 1;
        }
        assert!(
            carried == neutral.abilities.len(),
            "a neutral kind carries an ability its script does not list"
        );
    }
};

/// What one upgrade interval adds to any neutral: Dota's `neutral_upgrade`.
const NEUTRAL_GROWTH: Growth = Growth {
    hp: rules::NEUTRAL_UPGRADE_HP,
    damage: rules::NEUTRAL_UPGRADE_DAMAGE,
    armor_halves: rules::NEUTRAL_UPGRADE_ARMOR_HALVES,
    attack_speed: rules::NEUTRAL_UPGRADE_ATTACK_SPEED,
    gold_halves: 2 * rules::NEUTRAL_UPGRADE_GOLD,
    gold_alternates: true,
    xp: rules::NEUTRAL_UPGRADE_XP,
    ..NO_GROWTH
};

/// What every neutral shares, so an entry names only its own numbers.
const NEUTRAL_BASE: UnitDef = UnitDef {
    kind: UnitKind::CreepNeutral,
    per_upgrade: NEUTRAL_GROWTH,
    ..BARE_UNIT
};

/// How a unit attacks: its `AttackCapabilities`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitAttack {
    /// `DOTA_UNIT_CAP_NO_ATTACK`.
    None,
    /// `DOTA_UNIT_CAP_MELEE_ATTACK`.
    Melee,
    /// `DOTA_UNIT_CAP_RANGED_ATTACK`.
    Ranged,
}

/// How a unit moves: its `MovementCapabilities`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitMovement {
    /// `DOTA_UNIT_CAP_MOVE_GROUND`.
    Ground,
    /// `DOTA_UNIT_CAP_MOVE_FLY`.
    Fly,
}

/// The hull a unit moves with: its `BoundsHullName`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitHull {
    /// `DOTA_HULL_SIZE_SMALL`.
    Small,
    /// `DOTA_HULL_SIZE_REGULAR`.
    Regular,
    /// `DOTA_HULL_SIZE_HERO`.
    Hero,
}

/// One neutral unit as its script has it, in the script's own units.
#[derive(Clone, Copy, Debug)]
pub struct NeutralUnitData {
    /// Its Dota key.
    pub key: &'static str,
    /// Whether the patch the table was read from gives the unit to camps: false for a unit the
    /// registry retired, kept to be attached again.
    pub offered: bool,
    pub attack: UnitAttack,
    pub movement: UnitMovement,
    /// `BoundsHullName`, if the unit names one.
    pub hull: Option<UnitHull>,
    /// The abilities of its slots, in slot order, empty slots left out.
    pub abilities: &'static [&'static str],
    /// Its numeric keys.
    pub numbers: ScriptData,
}

/// Regeneration of `rate` points a second, a tick's worth.
const fn regen(rate: Dec) -> Fixed {
    Fixed::from_ratio(rate.0, 1000 * rules::TICKS_PER_SECOND as i32)
}

/// The definition a unit's script gives, every number looked up by its key (the importer writes
/// the game's default of a key a script leaves out into the row, with its reason): a unit that
/// does not attack has no attack numbers.
const fn unit_def(data: &NeutralUnitData) -> UnitDef {
    let n = &data.numbers;
    let attacks = !matches!(data.attack, UnitAttack::None);
    let flies = matches!(data.movement, UnitMovement::Fly);
    let (collision, bound) = if flies {
        // A flier has no body on the ground.
        (0, 0)
    } else {
        match data.hull {
            None | Some(UnitHull::Hero) => (rules::NEUTRAL_COLLISION, rules::NEUTRAL_BOUND),
            Some(UnitHull::Small) => (rules::RANGED_CREEP_COLLISION, rules::RANGED_CREEP_BOUND),
            Some(UnitHull::Regular) => panic!("no collision size is known for a regular hull"),
        }
    };
    let (damage, damage_spread) = if attacks {
        let least = n.first("AttackDamageMin").int();
        (least, n.first("AttackDamageMax").int() - least)
    } else {
        (0, 0)
    };
    UnitDef {
        max_hp: n.first("StatusHealth").int(),
        hp_regen: regen(n.first("StatusHealthRegen")),
        max_mana: n.first("StatusMana").int(),
        mana_regen: regen(n.first("StatusManaRegen")),
        damage,
        damage_spread,
        attack_range: if attacks {
            n.first("AttackRange").int()
        } else {
            0
        },
        acquisition: if attacks {
            n.first("AttackAcquisitionRange").int()
        } else {
            0
        },
        attack_time: if attacks {
            n.first("AttackRate").0 as u32
        } else {
            0
        },
        attack_speed: n.first("BaseAttackSpeed").int(),
        attack_point: if attacks {
            n.first("AttackAnimationPoint").0 as u32
        } else {
            0
        },
        attack_backswing: if attacks {
            n.first("AttackBackswing").0 as u32
        } else {
            0
        },
        projectile_speed: if matches!(data.attack, UnitAttack::Ranged) {
            Some(n.first("ProjectileSpeed").int())
        } else {
            None
        },
        armor: n.first("ArmorPhysical").int(),
        magic_resist_pct: n.first("MagicalResistance").int(),
        move_speed: n.first("MovementSpeed").int(),
        turn_rate: turn_rate_brads(n.first("MovementTurnRate")),
        day_vision: n.first("VisionDaytimeRange").int(),
        night_vision: n.first("VisionNighttimeRange").int(),
        flies,
        collision,
        bound,
        ancient: n.first("IsAncient").int() == 1,
        bounty_gold: GoldRange::new(
            n.first("BountyGoldMin").int(),
            n.first("BountyGoldMax").int(),
        ),
        bounty_xp: n.first("BountyXP").int(),
        ..NEUTRAL_BASE
    }
}

/// An entry, its kind given by its place and its numbers by its script.
const fn neutral(
    kind: NeutralKind,
    data: &'static NeutralUnitData,
    abilities: &'static [NeutralAbility],
) -> NeutralDef {
    let unit = data.key;
    let def = unit_def(data);
    NeutralDef {
        kind,

        unit,
        abilities,
        growth: Some(crate::NeutralGrowth {
            ability: NeutralAbility::NeutralUpgrade,
            clock: crate::GrowthClock {
                first_after_horn: rules::NEUTRAL_UPGRADE_PERIOD_TICKS,
                period: rules::NEUTRAL_UPGRADE_PERIOD_TICKS,
                cap: rules::NEUTRAL_UPGRADE_CAP as u32,
            },
        }),
        home: &JUNGLE_HOME,
        offered: data.offered,
        def,
    }
}

/// Every kind, each carrying its kind; [`NEUTRALS`] places them by it.
const NEUTRAL_ROWS: &[NeutralDef] = &[
    neutral(
        NeutralKind::Kobold,
        &script::NPC_DOTA_NEUTRAL_KOBOLD,
        &[A::ProspectingAura],
    ),
    neutral(
        NeutralKind::KoboldTunneler,
        &script::NPC_DOTA_NEUTRAL_KOBOLD_TUNNELER,
        &[A::StealWeaponCast],
    ),
    neutral(
        NeutralKind::KoboldTaskmaster,
        &script::NPC_DOTA_NEUTRAL_KOBOLD_TASKMASTER,
        &[A::SpeedAura],
    ),
    neutral(
        NeutralKind::ForestTrollBerserker,
        &script::NPC_DOTA_NEUTRAL_FOREST_TROLL_BERSERKER,
        &[A::Piercing],
    ),
    neutral(
        NeutralKind::ForestTrollHighPriest,
        &script::NPC_DOTA_NEUTRAL_FOREST_TROLL_HIGH_PRIEST,
        &[A::Heal, A::HealAmplificationAura, A::Piercing],
    ),
    neutral(
        NeutralKind::GnollAssassin,
        &script::NPC_DOTA_NEUTRAL_GNOLL_ASSASSIN,
        &[A::EnvenomedWeapon, A::Piercing],
    ),
    neutral(
        NeutralKind::FelBeast,
        &script::NPC_DOTA_NEUTRAL_FEL_BEAST,
        &[A::Vex],
    ),
    neutral(
        NeutralKind::Ghost,
        &script::NPC_DOTA_NEUTRAL_GHOST,
        &[A::FrostAttack, A::Piercing],
    ),
    neutral(
        NeutralKind::HarpyScout,
        &script::NPC_DOTA_NEUTRAL_HARPY_SCOUT,
        &[A::TakeOff, A::Piercing],
    ),
    neutral(
        NeutralKind::HarpyStorm,
        &script::NPC_DOTA_NEUTRAL_HARPY_STORM,
        &[A::ChainLightning, A::Piercing],
    ),
    neutral(
        NeutralKind::CentaurOutrunner,
        &script::NPC_DOTA_NEUTRAL_CENTAUR_OUTRUNNER,
        &[A::CloakAura],
    ),
    neutral(
        NeutralKind::CentaurKhan,
        &script::NPC_DOTA_NEUTRAL_CENTAUR_KHAN,
        &[A::WarStomp],
    ),
    neutral(
        NeutralKind::GiantWolf,
        &script::NPC_DOTA_NEUTRAL_GIANT_WOLF,
        &[A::Intimidate],
    ),
    neutral(
        NeutralKind::AlphaWolf,
        &script::NPC_DOTA_NEUTRAL_ALPHA_WOLF,
        &[A::CriticalStrike, A::PackleadersAura],
    ),
    neutral(
        NeutralKind::SatyrTrickster,
        &script::NPC_DOTA_NEUTRAL_SATYR_TRICKSTER,
        &[A::Purge],
    ),
    neutral(
        NeutralKind::SatyrSoulstealer,
        &script::NPC_DOTA_NEUTRAL_SATYR_SOULSTEALER,
        &[A::ManaBurn, A::ManaAura],
    ),
    neutral(
        NeutralKind::SatyrHellcaller,
        &script::NPC_DOTA_NEUTRAL_SATYR_HELLCALLER,
        &[A::Shockwave, A::UnholyAura],
    ),
    neutral(
        NeutralKind::OgreMauler,
        &script::NPC_DOTA_NEUTRAL_OGRE_MAULER,
        &[A::OgreSmash],
    ),
    neutral(
        NeutralKind::OgreMagi,
        &script::NPC_DOTA_NEUTRAL_OGRE_MAGI,
        &[A::IceArmor],
    ),
    neutral(
        NeutralKind::MudGolem,
        &script::NPC_DOTA_NEUTRAL_MUD_GOLEM,
        &[A::HurlBigBoulder, A::ShardSplit],
    ),
    neutral(
        NeutralKind::MudGolemSplit,
        &script::NPC_DOTA_NEUTRAL_MUD_GOLEM_SPLIT,
        &[A::HurlBoulder],
    ),
    neutral(
        NeutralKind::PolarFurbolgChampion,
        &script::NPC_DOTA_NEUTRAL_POLAR_FURBOLG_CHAMPION,
        &[A::SwiftnessAura, A::DeathThroeRush],
    ),
    neutral(
        NeutralKind::PolarFurbolgUrsaWarrior,
        &script::NPC_DOTA_NEUTRAL_POLAR_FURBOLG_URSA_WARRIOR,
        &[A::ThunderClap, A::DeathThroePower],
    ),
    neutral(
        NeutralKind::Wildkin,
        &script::NPC_DOTA_NEUTRAL_WILDKIN,
        &[A::Tornado],
    ),
    neutral(
        NeutralKind::EnragedWildkin,
        &script::NPC_DOTA_NEUTRAL_ENRAGED_WILDKIN,
        &[A::Hurricane, A::ToughnessAura],
    ),
    neutral(
        NeutralKind::DarkTroll,
        &script::NPC_DOTA_NEUTRAL_DARK_TROLL,
        &[A::Ensnare, A::Piercing],
    ),
    neutral(
        NeutralKind::DarkTrollWarlord,
        &script::NPC_DOTA_NEUTRAL_DARK_TROLL_WARLORD,
        &[A::RaiseDead],
    ),
    neutral(
        NeutralKind::WarpineRaider,
        &script::NPC_DOTA_NEUTRAL_WARPINE_RAIDER,
        &[A::SnowballShot],
    ),
    neutral(
        NeutralKind::BlackDrake,
        &script::NPC_DOTA_NEUTRAL_BLACK_DRAKE,
        &[A::MagicAmplificationAura],
    ),
    neutral(
        NeutralKind::BlackDragon,
        &script::NPC_DOTA_NEUTRAL_BLACK_DRAGON,
        &[A::SearingSwoop, A::SplashAttack, A::DragonhideAura],
    ),
    neutral(
        NeutralKind::RockGolem,
        &script::NPC_DOTA_NEUTRAL_ROCK_GOLEM,
        &[A::WeakeningAura],
    ),
    neutral(
        NeutralKind::GraniteGolem,
        &script::NPC_DOTA_NEUTRAL_GRANITE_GOLEM,
        &[A::GraniteAura],
    ),
    neutral(
        NeutralKind::SmallThunderLizard,
        &script::NPC_DOTA_NEUTRAL_SMALL_THUNDER_LIZARD,
        &[A::WarDrumsAura],
    ),
    neutral(
        NeutralKind::BigThunderLizard,
        &script::NPC_DOTA_NEUTRAL_BIG_THUNDER_LIZARD,
        &[A::Slam, A::Frenzy],
    ),
    neutral(
        NeutralKind::FrostbittenGolem,
        &script::NPC_DOTA_NEUTRAL_FROSTBITTEN_GOLEM,
        &[A::TimeWarpAura],
    ),
    neutral(
        NeutralKind::IceShaman,
        &script::NPC_DOTA_NEUTRAL_ICE_SHAMAN,
        &[A::IcefireBomb],
    ),
    neutral(
        NeutralKind::ProwlerAcolyte,
        &script::NPC_DOTA_NEUTRAL_PROWLER_ACOLYTE,
        &[A::ProwlerAura],
    ),
    neutral(
        NeutralKind::ProwlerShaman,
        &script::NPC_DOTA_NEUTRAL_PROWLER_SHAMAN,
        &[A::Desecrate, A::Petrify],
    ),
    neutral(
        NeutralKind::Tadpole,
        &script::NPC_DOTA_NEUTRAL_TADPOLE,
        &[A::RiverbornAura],
    ),
    neutral(
        NeutralKind::Froglet,
        &script::NPC_DOTA_NEUTRAL_FROGLET,
        &[A::ArmOfTheDeep, A::RiverbornAura],
    ),
    neutral(
        NeutralKind::GrownFrog,
        &script::NPC_DOTA_NEUTRAL_GROWN_FROG,
        &[A::TendrilsOfTheDeep, A::RiverbornAura],
    ),
    neutral(
        NeutralKind::AncientFrog,
        &script::NPC_DOTA_NEUTRAL_ANCIENT_FROG,
        &[A::CongregationsOfTheDeep, A::RiverbornAura],
    ),
    neutral(
        NeutralKind::FrogletMage,
        &script::NPC_DOTA_NEUTRAL_FROGLET_MAGE,
        &[A::WaterBubbleSmall, A::RiverbornAura],
    ),
    neutral(
        NeutralKind::GrownFrogMage,
        &script::NPC_DOTA_NEUTRAL_GROWN_FROG_MAGE,
        &[A::WaterBubbleMedium, A::RiverbornAura],
    ),
    neutral(
        NeutralKind::AncientFrogMage,
        &script::NPC_DOTA_NEUTRAL_ANCIENT_FROG_MAGE,
        &[A::WaterBubbleLarge, A::RiverbornAura],
    ),
    neutral(
        NeutralKind::SkeletonWarrior,
        &script::NPC_DOTA_DARK_TROLL_WARLORD_SKELETON_WARRIOR,
        &[A::Rally],
    ),
    NeutralDef {
        growth: None,
        ..neutral(
            NeutralKind::WildkinTornado,
            &script::NPC_DOTA_ENRAGED_WILDKIN_TORNADO,
            &[A::Tempest],
        )
    },
    NeutralDef {
        kind: NeutralKind::Roshan,

        unit: "npc_dota_roshan",
        abilities: &[
            A::RevengeRoar,
            A::RoshanSlam,
            A::Bash,
            A::SpellBlock,
            A::GrabAndThrow,
            A::StrengthOfTheImmortal,
            A::Devotion,
        ],
        growth: Some(crate::NeutralGrowth {
            ability: NeutralAbility::StrengthOfTheImmortal,
            clock: crate::GrowthClock {
                first_after_horn: 60 * rules::TICKS_PER_SECOND,
                period: 60 * rules::TICKS_PER_SECOND,
                cap: u32::MAX,
            },
        }),
        home: &crate::ROSHAN_HOME,
        offered: true,
        def: crate::ROSHAN,
    },
    neutral(
        NeutralKind::Radigull,
        &script::NPC_DOTA_NEUTRAL_RADIGULL,
        &[A::Eggstremism],
    ),
    neutral(
        NeutralKind::OdobenusFootman,
        &script::NPC_DOTA_NEUTRAL_ODOBENUS_FOOTMAN,
        &[A::OdobenusBlight],
    ),
    neutral(
        NeutralKind::OdobenusFrostmage,
        &script::NPC_DOTA_NEUTRAL_ODOBENUS_FROSTMAGE,
        &[A::SnowballShot],
    ),
    neutral(
        NeutralKind::Eimermole,
        &script::NPC_DOTA_NEUTRAL_EIMERMOLE,
        &[A::MinersAura],
    ),
    neutral(
        NeutralKind::EimermoleForeman,
        &script::NPC_DOTA_NEUTRAL_EIMERMOLE_FOREMAN,
        &[A::MinersSlam],
    ),
    neutral(
        NeutralKind::OgreSeal,
        &script::NPC_DOTA_NEUTRAL_OGRE_SEAL,
        &[A::SealFlop],
    ),
    neutral(
        NeutralKind::OgreSealPup,
        &script::NPC_DOTA_NEUTRAL_OGRE_SEAL_PUP,
        &[A::SealDisapprovalAura],
    ),
    neutral(
        NeutralKind::KoboldTunnelerGold,
        &script::NPC_DOTA_NEUTRAL_KOBOLD_TUNNELER_GOLD,
        &[A::StealWeaponCast, A::GoldUpgrade],
    ),
    neutral(
        NeutralKind::GnollAssassinGold,
        &script::NPC_DOTA_NEUTRAL_GNOLL_ASSASSIN_GOLD,
        &[A::EnvenomedWeapon, A::Piercing, A::GoldUpgrade],
    ),
    neutral(
        NeutralKind::AlphaWolfGold,
        &script::NPC_DOTA_NEUTRAL_ALPHA_WOLF_GOLD,
        &[A::CriticalStrike, A::PackleadersAura, A::GoldUpgrade],
    ),
    neutral(
        NeutralKind::MudGolemGold,
        &script::NPC_DOTA_NEUTRAL_MUD_GOLEM_GOLD,
        &[A::HurlBigBoulder, A::ShardSplit, A::GoldUpgrade],
    ),
    neutral(
        NeutralKind::EnragedWildkinGold,
        &script::NPC_DOTA_NEUTRAL_ENRAGED_WILDKIN_GOLD,
        &[A::Hurricane, A::ToughnessAura, A::GoldUpgrade],
    ),
    neutral(
        NeutralKind::PolarFurbolgChampionGold,
        &script::NPC_DOTA_NEUTRAL_POLAR_FURBOLG_CHAMPION_GOLD,
        &[A::SwiftnessAura, A::DeathThroeRush, A::GoldUpgrade],
    ),
];

/// How many neutral ids the protocol has ever handed out.
pub const NEUTRAL_COUNT: usize = NEUTRAL_ROWS.len();

/// Every kind, indexed by [`NeutralKind`]: its [`NeutralId`].
pub static NEUTRALS: [NeutralDef; NEUTRAL_COUNT] =
    placed_by_id!(NEUTRAL_ROWS, NEUTRAL_COUNT, |row| row.kind as usize);
