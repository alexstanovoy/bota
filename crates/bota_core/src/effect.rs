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
    /// Whether it hurts whoever wears it: a debuff, apart from the buffs.
    pub harmful: bool,
    /// Whether the id is out of use for good: kept so that no other effect
    /// takes its number.
    pub retired: bool,
    /// Whether a basic dispel takes it off.
    pub dispellable: bool,
    /// Additional eligibility for strong dispels; basic dispels are always strong-dispellable.
    pub strong_dispellable: bool,
    /// Whether a hero keeps it through death, as Dota keeps a modifier that is
    /// not removed on death: what it has gathered.
    pub kept_through_death: bool,
    /// Whether no client is ever told of it, as Dota hides a modifier: its
    /// worth shows only in the stats of whatever wears it.
    pub hidden: bool,
    /// Independent lifetimes for each aura or world source.
    pub per_source: bool,
    /// Whether only the side of whatever wears it is told of it, as Dota
    /// tells the other side nothing of `modifier_illusion`.
    pub own_side_only: bool,
    /// Whether whatever wears it is gone when it runs out, as Dota's
    /// `modifier_kill` and `modifier_illusion` end their units.
    pub ends_its_wearer: bool,
    /// Whether another side is told of it only while it sees where its
    /// wearer stands: what a building both sides always know of shows
    /// through the fog, but not this, as Dota draws a particle in the fog
    /// only for whoever sees its spot.
    pub sighted_only: bool,
}

const fn buff(id: u16, key: &'static str) -> EffectDef {
    EffectDef {
        id: EffectId(id),
        key,

        harmful: false,
        retired: false,
        dispellable: false,
        strong_dispellable: false,
        kept_through_death: false,
        hidden: false,
        per_source: false,
        own_side_only: false,
        ends_its_wearer: false,
        sighted_only: false,
    }
}

const fn debuff(id: u16, key: &'static str) -> EffectDef {
    EffectDef {
        harmful: true,
        ..buff(id, key)
    }
}

/// A count of what a hero has gathered, kept through its deaths.
const fn gathered(id: u16, key: &'static str) -> EffectDef {
    EffectDef {
        kept_through_death: true,
        ..buff(id, key)
    }
}

/// A modifier no client is told of.
const fn hidden(id: u16, key: &'static str) -> EffectDef {
    EffectDef {
        hidden: true,
        ..buff(id, key)
    }
}

/// A debuff a basic dispel takes off.
const fn dispellable_debuff(id: u16, key: &'static str) -> EffectDef {
    EffectDef {
        dispellable: true,
        ..debuff(id, key)
    }
}

/// A buff a basic dispel takes off.
const fn fleeting(id: u16, key: &'static str) -> EffectDef {
    EffectDef {
        dispellable: true,
        ..buff(id, key)
    }
}

/// Every effect, each carrying its id and its key; [`EFFECTS`] places them
/// by id.
const EFFECT_ROWS: &[EffectDef] = &[
    EffectDef {
        retired: true,
        ..buff(0, "bota_haste")
    },
    fleeting(1, "modifier_flask_healing"),
    fleeting(2, "modifier_clarity_potion"),
    buff(3, "modifier_fountain_aura_buff"),
    EffectDef {
        strong_dispellable: true,
        ..debuff(4, "modifier_stunned")
    },
    debuff(5, "bota_slowed"),
    debuff(6, "bota_burning"),
    fleeting(7, "modifier_courier_burst"),
    buff(8, "modifier_courier_shield"),
    fleeting(9, "modifier_item_phase_boots_active"),
    gathered(10, "modifier_pudge_innate_graft_flesh"),
    gathered(11, "modifier_nevermore_necromastery"),
    debuff(12, "modifier_nevermore_presence"),
    buff(13, "modifier_tower_armor_bonus"),
    buff(14, "modifier_flagbearer_creep_aura_effect"),
    debuff(15, "modifier_nevermore_shadowraze_debuff"),
    // Burns whoever wears it: the enemies in it and, never to death, its
    // owner.
    debuff(16, "modifier_pudge_rot"),
    EffectDef {
        strong_dispellable: true,
        ..debuff(17, "modifier_nevermore_requiem_fear")
    },
    debuff(18, "modifier_nevermore_requiem_slow"),
    fleeting(19, "modifier_nevermore_frenzy"),
    buff(20, "modifier_nevermore_feast_of_souls_linger"),
    debuff(21, "modifier_item_blood_grenade_debuff"),
    EffectDef {
        dispellable: true,
        ..debuff(22, "modifier_rooted")
    },
    EffectDef {
        dispellable: true,
        ..debuff(23, "modifier_disarmed")
    },
    EffectDef {
        dispellable: true,
        ..debuff(24, "modifier_break")
    },
    debuff(25, "modifier_kobold_disarm"),
    buff(26, "modifier_kobold_taskmaster_speed_aura_bonus"),
    debuff(27, "modifier_gnoll_assassin_envenomed_weapon_poison"),
    debuff(28, "modifier_ghost_frost_attack_slow"),
    buff(29, "modifier_mudgolem_cloak_aura_bonus"),
    debuff(30, "modifier_giant_wolf_intimidate"),
    buff(31, "modifier_alpha_wolf_command_aura_bonus"),
    buff(32, "modifier_forest_troll_high_priest_mana_aura_bonus"),
    buff(33, "modifier_satyr_hellcaller_unholy_aura_bonus"),
    debuff(34, "modifier_mud_golem_rock_destroy"),
    buff(35, "modifier_centaur_khan_endurance_aura_bonus"),
    fleeting(36, "modifier_furbolg_enrage_attack_speed"),
    debuff(37, "modifier_polar_furbolg_ursa_warrior_thunder_clap"),
    fleeting(38, "modifier_furbolg_enrage_damage"),
    buff(39, "modifier_enraged_wildkin_toughness_aura_bonus"),
    debuff(40, "modifier_black_drake_magic_amplification"),
    buff(41, "modifier_black_dragon_dragonhide_aura_bonus"),
    debuff(42, "modifier_ancient_rock_golem_weakening"),
    buff(43, "modifier_granite_golem_hp_aura_bonus"),
    buff(44, "modifier_big_thunder_lizard_wardrums_aura"),
    debuff(45, "modifier_big_thunder_lizard_slam"),
    fleeting(46, "modifier_big_thunder_lizard_frenzy"),
    buff(47, "modifier_frostbitten_golem_time_warp_aura_bonus"),
    buff(48, "modifier_spawnlord_aura_bonus"),
    debuff(49, "modifier_spawnlord_master_stomp"),
    debuff(50, "modifier_spawnlord_master_freeze_root"),
    buff(51, "modifier_frogmen_riverborn_aura_bonus"),
    buff(52, "modifier_hill_troll_rally"),
    debuff(53, "modifier_tornado_tempest_debuff"),
    fleeting(54, "modifier_frogmen_water_bubble"),
    debuff(55, "modifier_satyr_trickster_purge"),
    EffectDef {
        per_source: true,
        ..buff(56, "modifier_fountain_glyph")
    },
    buff(57, "modifier_pudge_flesh_heap_block"),
    EffectDef {
        strong_dispellable: true,
        ..debuff(58, "modifier_pudge_dismember")
    },
    buff(59, "modifier_backdoor_protection_active"),
    EffectDef {
        own_side_only: true,
        ends_its_wearer: true,
        ..buff(60, "modifier_illusion")
    },
    EffectDef {
        ends_its_wearer: true,
        ..buff(61, "modifier_kill")
    },
    hidden(62, "modifier_neutral_upgrade"),
    buff(63, "modifier_courier_flying"),
    buff(64, "modifier_fountain_invulnerability"),
    hidden(65, "modifier_fountain_passive"),
    debuff(66, "modifier_fountain_fury_swipes_damage_increase"),
    buff(67, "modifier_lamp_on"),
    buff(68, "modifier_lamp_off"),
    buff(69, "modifier_watch_tower_invulnerable"),
    buff(70, "bota_shrine_ready"),
    buff(71, "modifier_rune_haste"),
    buff(72, "modifier_rune_doubledamage"),
    buff(73, "modifier_rune_arcane"),
    buff(74, "modifier_rune_regen"),
    buff(75, "modifier_rune_invis"),
    buff(76, "modifier_rune_shield"),
    fleeting(77, "modifier_bottle_regeneration"),
    dispellable_debuff(78, "modifier_blight_stone_buff"),
    dispellable_debuff(79, "modifier_item_orb_of_venom_slow"),
    dispellable_debuff(80, "modifier_orb_of_frost_debuff"),
    EffectDef {
        dispellable: true,
        ..buff(81, "modifier_aegis_regen")
    },
    buff(82, "modifier_roshan_devotion"),
    hidden(83, "modifier_roshan_inherent_buffs"),
    hidden(84, "modifier_roshan_devotion_aura"),
    hidden(85, "modifier_roshan_spell_block"),
    EffectDef {
        dispellable: true,
        ..debuff(86, "modifier_roshan_slam")
    },
    EffectDef {
        strong_dispellable: true,
        ..debuff(87, "modifier_bashed")
    },
    hidden(88, "modifier_roshan_revengeroar_windup"),
    hidden(89, "modifier_roshan_revengeroar_aura"),
    debuff(90, "modifier_roshan_revengeroar_disarm"),
    EffectDef {
        dispellable: true,
        ..debuff(91, "modifier_roshan_revengeroar")
    },
    EffectDef {
        dispellable: true,
        ..debuff(92, "modifier_silence")
    },
    debuff(93, "modifier_roshan_grab"),
    debuff(94, "modifier_roshan_grab_thrown"),
    buff(95, "modifier_roshan_moving"),
    debuff(96, "modifier_knockback"),
    EffectDef {
        per_source: true,
        ..hidden(97, "modifier_roshan_moving_knockback_timeout")
    },
    fleeting(98, "modifier_ghost_state"),
    // Both gates glow while a warp is channelled through either [Liquipedia
    // and the Dota 2 Wiki, Twin Gate: "the other Twin Gate plays its
    // channeling animation and channeling sounds as well"]; each glow is seen
    // by whoever sees that gate: "Twin Gates are always visible through the
    // Fog of War, though enemies cannot see when a Twin Gate gets used without
    // having vision over them" [Liquipedia, Buildings/Neutral Buildings]. A
    // side watching the exit sees it glow, not who channels.
    EffectDef {
        sighted_only: true,
        ..buff(99, "modifier_twin_gate_fx")
    },
    hidden(100, "modifier_pudge_meat_hook_followthrough"),
    buff(101, "modifier_item_headdress_aura"),
    buff(102, "modifier_item_ring_of_basilius_aura"),
    buff(103, "modifier_item_buckler_effect"),
    buff(104, "modifier_item_mekansm_aura"),
    buff(105, "modifier_item_mekansm_noheal"),
    buff(106, "modifier_item_ancient_janggo_aura"),
    EffectDef {
        per_source: true,
        ..buff(107, "modifier_item_ancient_janggo_active")
    },
    buff(108, "modifier_item_pipe_aura"),
    buff(109, "modifier_item_pipe_barrier"),
    buff(110, "modifier_item_pipe_debuff"),
    buff(111, "modifier_item_guardian_greaves_aura"),
    hidden(112, "modifier_item_guardian_greaves_bonus"),
    buff(113, "modifier_item_assault_positive_armor"),
    debuff(114, "modifier_item_assault_negative_armor"),
    buff(115, "modifier_item_vladmir_aura"),
    debuff(116, "modifier_item_radiance_debuff"),
    debuff(117, "modifier_item_shivas_guard_aura"),
    dispellable_debuff(118, "modifier_item_shivas_guard_blast"),
    buff(119, "modifier_item_boots_of_bearing_aura"),
    EffectDef {
        per_source: true,
        ..buff(120, "modifier_item_boots_of_bearing_active")
    },
    buff(121, "modifier_item_boots_of_bearing_active_bonus"),
    hidden(122, "bota_spell_immunity"),
    debuff(123, "modifier_item_crellas_crozier_debuff"),
    buff(124, "modifier_item_crellas_crozier_active"),
    EffectDef {
        dispellable: true,
        ..debuff(125, "modifier_item_crellas_crozier_movespeed_debuff")
    },
    EffectDef {
        dispellable: true,
        ..buff(126, "modifier_item_crellas_crozier_movespeed_buff")
    },
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
