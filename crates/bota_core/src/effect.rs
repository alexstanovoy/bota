//! Every timed effect a unit may show on the wire, and how a client treats
//! it.

use bota_proto::EffectId;

use crate::table::placed_by_id;

/// One entry of the effect list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectDef {
    /// The id it answers to on the wire, which [`EFFECTS`] places it at.
    pub id: EffectId,
    /// Dota's internal name of the modifier it is, its line of the id
    /// registry; a `bota_` key for one Dota has no modifier for.
    pub key: &'static str,
    /// What it is called.
    pub name: &'static str,
    /// Whether it hurts whoever wears it: a debuff, apart from the buffs.
    pub harmful: bool,
    /// Whether the id is out of use for good: kept so that no other effect
    /// takes its number.
    pub retired: bool,
    /// Whether a basic dispel takes it off.
    pub dispellable: bool,
    /// Whether a hero keeps it through death, as Dota keeps a modifier that is
    /// not removed on death: what it has gathered.
    pub kept_through_death: bool,
    /// Whether no client is ever told of it, as Dota hides a modifier: its
    /// worth shows only in the stats of whatever wears it.
    pub hidden: bool,
    /// Whether only the side of whatever wears it is told of it, as Dota
    /// tells the other side nothing of `modifier_illusion`.
    pub own_side_only: bool,
    /// Whether whatever wears it is gone when it runs out, as Dota's
    /// `modifier_kill` and `modifier_illusion` end their units.
    pub ends_its_wearer: bool,
}

const fn buff(id: u16, key: &'static str, name: &'static str) -> EffectDef {
    EffectDef {
        id: EffectId(id),
        key,
        name,
        harmful: false,
        retired: false,
        dispellable: false,
        kept_through_death: false,
        hidden: false,
        own_side_only: false,
        ends_its_wearer: false,
    }
}

const fn debuff(id: u16, key: &'static str, name: &'static str) -> EffectDef {
    EffectDef {
        harmful: true,
        ..buff(id, key, name)
    }
}

/// A count of what a hero has gathered, kept through its deaths.
const fn gathered(id: u16, key: &'static str, name: &'static str) -> EffectDef {
    EffectDef {
        kept_through_death: true,
        ..buff(id, key, name)
    }
}

/// A modifier no client is told of.
const fn hidden(id: u16, key: &'static str, name: &'static str) -> EffectDef {
    EffectDef {
        hidden: true,
        ..buff(id, key, name)
    }
}

/// A buff a basic dispel takes off.
const fn fleeting(id: u16, key: &'static str, name: &'static str) -> EffectDef {
    EffectDef {
        dispellable: true,
        ..buff(id, key, name)
    }
}

/// Every effect, each carrying its id and its key; [`EFFECTS`] places them
/// by id.
const EFFECT_ROWS: &[EffectDef] = &[
    EffectDef {
        retired: true,
        ..buff(0, "bota_haste", "Haste")
    },
    fleeting(1, "modifier_flask_healing", "Mending"),
    fleeting(2, "modifier_clarity_potion", "Clarity"),
    buff(3, "modifier_fountain_aura_buff", "Fountain"),
    debuff(4, "modifier_stunned", "Stunned"),
    debuff(5, "bota_slowed", "Slowed"),
    debuff(6, "bota_burning", "Burning"),
    fleeting(7, "modifier_courier_burst", "Hastened"),
    buff(8, "modifier_courier_shield", "Shielded"),
    fleeting(9, "modifier_item_phase_boots_active", "Phased"),
    gathered(10, "modifier_pudge_innate_graft_flesh", "FleshHeap"),
    gathered(11, "modifier_nevermore_necromastery", "Souls"),
    debuff(12, "modifier_nevermore_presence", "ArmorBroken"),
    buff(13, "modifier_tower_armor_bonus", "Guarded"),
    buff(14, "modifier_flagbearer_creep_aura_effect", "Inspired"),
    debuff(15, "modifier_nevermore_shadowraze_debuff", "Shadowraze"),
    // Burns whoever wears it: the enemies in it and, never to death, its
    // owner.
    debuff(16, "modifier_pudge_rot", "Rot"),
    debuff(17, "modifier_nevermore_requiem_fear", "Feared"),
    debuff(18, "modifier_nevermore_requiem_slow", "MagicBroken"),
    fleeting(19, "modifier_nevermore_frenzy", "Feasting"),
    buff(
        20,
        "modifier_nevermore_feast_of_souls_linger",
        "FeastLinger",
    ),
    debuff(21, "modifier_item_blood_grenade_debuff", "BloodGrenade"),
    debuff(22, "modifier_rooted", "Rooted"),
    debuff(23, "modifier_disarmed", "Disarmed"),
    debuff(24, "modifier_break", "Broken"),
    debuff(25, "modifier_kobold_disarm", "StealWeapon"),
    buff(
        26,
        "modifier_kobold_taskmaster_speed_aura_bonus",
        "SpeedAura",
    ),
    debuff(
        27,
        "modifier_gnoll_assassin_envenomed_weapon_poison",
        "EnvenomedWeapon",
    ),
    debuff(28, "modifier_ghost_frost_attack_slow", "FrostAttack"),
    buff(29, "modifier_mudgolem_cloak_aura_bonus", "CloakAura"),
    debuff(30, "modifier_giant_wolf_intimidate", "Intimidate"),
    buff(
        31,
        "modifier_alpha_wolf_command_aura_bonus",
        "PackleadersAura",
    ),
    buff(
        32,
        "modifier_forest_troll_high_priest_mana_aura_bonus",
        "ManaAura",
    ),
    buff(
        33,
        "modifier_satyr_hellcaller_unholy_aura_bonus",
        "UnholyAura",
    ),
    debuff(34, "modifier_mud_golem_rock_destroy", "ShardSplit"),
    buff(
        35,
        "modifier_centaur_khan_endurance_aura_bonus",
        "SwiftnessAura",
    ),
    fleeting(36, "modifier_furbolg_enrage_attack_speed", "DeathThroeRush"),
    debuff(
        37,
        "modifier_polar_furbolg_ursa_warrior_thunder_clap",
        "ThunderClap",
    ),
    fleeting(38, "modifier_furbolg_enrage_damage", "DeathThroePower"),
    buff(
        39,
        "modifier_enraged_wildkin_toughness_aura_bonus",
        "ToughnessAura",
    ),
    debuff(
        40,
        "modifier_black_drake_magic_amplification",
        "MagicAmplificationAura",
    ),
    buff(
        41,
        "modifier_black_dragon_dragonhide_aura_bonus",
        "DragonhideAura",
    ),
    debuff(42, "modifier_ancient_rock_golem_weakening", "WeakeningAura"),
    buff(43, "modifier_granite_golem_hp_aura_bonus", "GraniteAura"),
    buff(
        44,
        "modifier_big_thunder_lizard_wardrums_aura",
        "WarDrumsAura",
    ),
    debuff(45, "modifier_big_thunder_lizard_slam", "Slam"),
    fleeting(46, "modifier_big_thunder_lizard_frenzy", "Frenzy"),
    buff(
        47,
        "modifier_frostbitten_golem_time_warp_aura_bonus",
        "TimeWarpAura",
    ),
    buff(48, "modifier_spawnlord_aura_bonus", "ProwlerAura"),
    debuff(49, "modifier_spawnlord_master_stomp", "Desecrate"),
    debuff(50, "modifier_spawnlord_master_freeze_root", "Petrify"),
    buff(51, "modifier_frogmen_riverborn_aura_bonus", "RiverbornAura"),
    buff(52, "modifier_hill_troll_rally", "Rally"),
    debuff(53, "modifier_tornado_tempest_debuff", "Tempest"),
    fleeting(54, "modifier_frogmen_water_bubble", "WaterBubble"),
    debuff(55, "modifier_satyr_trickster_purge", "Purge"),
    buff(56, "modifier_fountain_glyph", "Fortified"),
    buff(57, "modifier_pudge_flesh_heap_block", "MeatShield"),
    debuff(58, "modifier_pudge_dismember", "Dismembered"),
    buff(
        59,
        "modifier_backdoor_protection_active",
        "BackdoorProtection",
    ),
    EffectDef {
        own_side_only: true,
        ends_its_wearer: true,
        ..buff(60, "modifier_illusion", "Illusion")
    },
    EffectDef {
        ends_its_wearer: true,
        ..buff(61, "modifier_kill", "TimedLife")
    },
    hidden(62, "modifier_neutral_upgrade", "NeutralUpgrade"),
    buff(63, "modifier_courier_flying", "Flying"),
    buff(
        64,
        "modifier_fountain_invulnerability",
        "FountainInvulnerable",
    ),
    hidden(65, "modifier_fountain_passive", "FountainPassive"),
    debuff(
        66,
        "modifier_fountain_fury_swipes_damage_increase",
        "FountainDamage",
    ),
    buff(67, "modifier_lamp_on", "VisionGrant"),
    buff(68, "modifier_lamp_off", "LampOff"),
    buff(69, "modifier_watch_tower_invulnerable", "CaptureGuard"),
    buff(70, "bota_shrine_ready", "ShrineReady"),
    buff(71, "modifier_rune_haste", "RuneHaste"),
    buff(72, "modifier_rune_doubledamage", "RuneAmplifyDamage"),
    buff(73, "modifier_rune_arcane", "RuneArcane"),
    buff(74, "modifier_rune_regen", "RuneRegeneration"),
    buff(75, "modifier_rune_invis", "RuneInvisibility"),
    buff(76, "modifier_rune_shield", "RuneShield"),
    fleeting(77, "modifier_bottle_regeneration", "BottleRegeneration"),
];

/// How many effect ids the protocol has ever handed out, retired ones
/// included: every table indexed by [`EffectId`] is this long.
pub const EFFECT_COUNT: usize = EFFECT_ROWS.len();

/// Every effect, indexed by [`EffectId`].
pub const EFFECTS: [EffectDef; EFFECT_COUNT] =
    placed_by_id!(EFFECT_ROWS, EFFECT_COUNT, |row| row.id.0 as usize);

/// What one effect is, or nothing if the id is retired or was never handed
/// out.
#[inline]
pub fn effect_def(id: EffectId) -> Option<&'static EffectDef> {
    EFFECTS.get(usize::from(id.0)).filter(|def| !def.retired)
}
