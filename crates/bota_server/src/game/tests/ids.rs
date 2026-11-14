//! Every effect the server hands out is shown under its registered id.

use bota_core::{ModifierKind, NEUTRAL_ABILITIES, effect_def, spell_effect};
use bota_proto::{DamageKind, EffectId, Fixed, IdSpace, RegisteredId, registered};

/// One modifier of every kind. A kind added to the enum fails to compile in
/// the match below until it is listed here as well.
fn every_modifier() -> Vec<ModifierKind> {
    let mut every = vec![
        ModifierKind::SpellImmunity,
        ModifierKind::Journey {
            ability: bota_core::NeutralAbility::StrengthOfTheImmortal,
            stops: 0,
            throws: 0,
        },
        ModifierKind::DelayedSpell {
            ability: bota_core::NeutralAbility::RevengeRoar,
            level: 1,
        },
        ModifierKind::AuraEmitter {
            ability: bota_core::NeutralAbility::RevengeRoar,
            level: 1,
        },
        ModifierKind::Control {
            control: bota_core::Control::Disarm,
            effect: EffectId(90),
        },
        ModifierKind::Control {
            control: bota_core::Control::Silence,
            effect: EffectId(92),
        },
        ModifierKind::Control {
            control: bota_core::Control::Stun,
            effect: EffectId(87),
        },
        ModifierKind::BonusArmor {
            armor: Fixed::ONE,
            effect: EffectId(82),
        },
        ModifierKind::Recovery {
            health_pct: 20,
            mana_pct: 20,
        },
        ModifierKind::Mending {
            per_tick: Fixed::ZERO,
            breaks: false,
        },
        ModifierKind::Clarity {
            per_tick: Fixed::ZERO,
            breaks: false,
        },
        ModifierKind::Fountain {
            hp_pct: 0,
            mana_pct: 0,
        },
        ModifierKind::Stunned,
        ModifierKind::Shielded,
        ModifierKind::Phased,
        ModifierKind::Flying,
        ModifierKind::FountainInvulnerable,
        ModifierKind::Slowed { pct: 0 },
        ModifierKind::Hastened { pct: 0 },
        ModifierKind::Haste { pct: 0 },
        ModifierKind::ArmorBroken { armor: Fixed::ZERO },
        ModifierKind::MagicBroken { pct: 0 },
        ModifierKind::Guarded {
            armor: 0,
            hp_per_second: 0,
        },
        ModifierKind::Inspired {
            hp_per_second: 0,
            magic_resist_pct: 0,
        },
        ModifierKind::Burning {
            amount: 0,
            kind: DamageKind::Pure,
            lethal: false,
        },
        ModifierKind::Shadowraze { stacks: 1 },
        ModifierKind::Rot { level: 0, since: 0 },
        ModifierKind::Rotting {
            level: 0,
            slow_pct: 14,
            since: 0,
        },
        ModifierKind::Feared,
        ModifierKind::BloodGrenade { since: 0 },
        ModifierKind::Rooted,
        ModifierKind::Disarmed,
        ModifierKind::Broken,
        ModifierKind::Dismembered {
            level: 0,
            bites: 6,
            channel_ms: 2750,
        },
        ModifierKind::MeatShield { amount: 8 },
        ModifierKind::BackdoorProtection,
        ModifierKind::Fortified,
        ModifierKind::TimedLife,
        ModifierKind::Illusion,
        ModifierKind::NeutralUpgrade { count: 1 },
        ModifierKind::FleshHeap { count: 1 },
        ModifierKind::Souls { count: 1 },
        ModifierKind::Feasting { level: 0, souls: 0 },
        ModifierKind::FeastLinger { souls: 1 },
        ModifierKind::FountainPassive,
        ModifierKind::FountainDamage { stacks: 1 },
        ModifierKind::VisionGrant {
            side: bota_proto::Team::Radiant,
            day: 0,
            night: 0,
        },
        ModifierKind::LampOff,
        ModifierKind::CaptureGuard,
        ModifierKind::ShrineReady,
        ModifierKind::RuneHaste,
        ModifierKind::RuneAmplifyDamage,
        ModifierKind::RuneArcane,
        ModifierKind::RuneRegeneration { damped: 0 },
        ModifierKind::RuneInvisibility,
        ModifierKind::RuneShield { left: 1 },
        ModifierKind::BottleRegeneration {
            hp_per_tick: 1,
            mana_per_tick: 1,
        },
        ModifierKind::Ethereal { pct: 30 },
        ModifierKind::TwinGateActive,
        ModifierKind::HookFollowthrough,
    ];
    // What every item that lays something lays.
    every.extend(
        bota_core::ITEMS
            .iter()
            .filter(|def| !def.retired && def.on_attack.is_some())
            .map(|def| ModifierKind::Laid {
                item: def.id,
                since: 0,
            }),
    );
    for item in bota_core::ITEMS.iter().filter(|item| !item.retired) {
        every.extend(
            item.modifiers
                .iter()
                .enumerate()
                .map(|(index, template)| template.kind(item.id, index as u8, 0, false)),
        );
    }
    for kind in &every {
        match kind {
            ModifierKind::Recovery { .. }
            | ModifierKind::Mending { .. }
            | ModifierKind::Clarity { .. }
            | ModifierKind::Fountain { .. }
            | ModifierKind::Stunned
            | ModifierKind::Shielded
            | ModifierKind::Phased
            | ModifierKind::Flying
            | ModifierKind::FountainInvulnerable
            | ModifierKind::FountainPassive
            | ModifierKind::FountainDamage { .. }
            | ModifierKind::Slowed { .. }
            | ModifierKind::Hastened { .. }
            | ModifierKind::Haste { .. }
            | ModifierKind::ArmorBroken { .. }
            | ModifierKind::MagicBroken { .. }
            | ModifierKind::Guarded { .. }
            | ModifierKind::Inspired { .. }
            | ModifierKind::Burning { .. }
            | ModifierKind::Shadowraze { .. }
            | ModifierKind::Rot { .. }
            | ModifierKind::Rotting { .. }
            | ModifierKind::Feared
            | ModifierKind::BloodGrenade { .. }
            | ModifierKind::Rooted
            | ModifierKind::Disarmed
            | ModifierKind::Broken
            | ModifierKind::BackdoorProtection
            | ModifierKind::Fortified
            | ModifierKind::TimedLife
            | ModifierKind::Illusion
            | ModifierKind::NeutralUpgrade { .. }
            | ModifierKind::FleshHeap { .. }
            | ModifierKind::Souls { .. }
            | ModifierKind::Feasting { .. }
            | ModifierKind::FeastLinger { .. }
            | ModifierKind::VisionGrant { .. }
            | ModifierKind::LampOff
            | ModifierKind::CaptureGuard
            | ModifierKind::ShrineReady
            | ModifierKind::RuneHaste
            | ModifierKind::RuneAmplifyDamage
            | ModifierKind::RuneArcane
            | ModifierKind::RuneRegeneration { .. }
            | ModifierKind::RuneInvisibility
            | ModifierKind::RuneShield { .. }
            | ModifierKind::BottleRegeneration { .. }
            | ModifierKind::Laid { .. }
            | ModifierKind::Ethereal { .. }
            | ModifierKind::TwinGateActive
            | ModifierKind::HookFollowthrough
            | ModifierKind::Spell { .. } => {}
            ModifierKind::Journey { .. }
            | ModifierKind::DelayedSpell { .. }
            | ModifierKind::AuraEmitter { .. }
            | ModifierKind::Control { .. }
            | ModifierKind::BonusArmor { .. }
            | ModifierKind::Dismembered { .. }
            | ModifierKind::MeatShield { .. }
            | ModifierKind::ItemBuff { .. }
            | ModifierKind::SpellImmunity => {}
        }
    }
    every
}

// Expected modifier identities, independent of the effect table under test.
const MODIFIER_KEYS: &[(&str, &str)] = &[
    ("Haste", "bota_haste"),
    ("Mending", "modifier_flask_healing"),
    ("Clarity", "modifier_clarity_potion"),
    ("Fountain", "modifier_fountain_aura_buff"),
    ("Stunned", "modifier_stunned"),
    ("Slowed", "bota_slowed"),
    ("Burning", "bota_burning"),
    ("Hastened", "modifier_courier_burst"),
    ("Shielded", "modifier_courier_shield"),
    ("Phased", "modifier_item_phase_boots_active"),
    ("FleshHeap", "modifier_pudge_innate_graft_flesh"),
    ("Souls", "modifier_nevermore_necromastery"),
    ("ArmorBroken", "modifier_nevermore_presence"),
    ("Guarded", "modifier_tower_armor_bonus"),
    ("Inspired", "modifier_flagbearer_creep_aura_effect"),
    ("Shadowraze", "modifier_nevermore_shadowraze_debuff"),
    ("Rot", "modifier_pudge_rot"),
    ("Feared", "modifier_nevermore_requiem_fear"),
    ("MagicBroken", "modifier_nevermore_requiem_slow"),
    ("Feasting", "modifier_nevermore_frenzy"),
    ("FeastLinger", "modifier_nevermore_feast_of_souls_linger"),
    ("BloodGrenade", "modifier_item_blood_grenade_debuff"),
    ("Rooted", "modifier_rooted"),
    ("Disarmed", "modifier_disarmed"),
    ("Broken", "modifier_break"),
    ("StealWeapon", "modifier_kobold_disarm"),
    ("SpeedAura", "modifier_kobold_taskmaster_speed_aura_bonus"),
    (
        "EnvenomedWeapon",
        "modifier_gnoll_assassin_envenomed_weapon_poison",
    ),
    ("FrostAttack", "modifier_ghost_frost_attack_slow"),
    ("CloakAura", "modifier_mudgolem_cloak_aura_bonus"),
    ("Intimidate", "modifier_giant_wolf_intimidate"),
    ("PackleadersAura", "modifier_alpha_wolf_command_aura_bonus"),
    (
        "ManaAura",
        "modifier_forest_troll_high_priest_mana_aura_bonus",
    ),
    ("UnholyAura", "modifier_satyr_hellcaller_unholy_aura_bonus"),
    ("ShardSplit", "modifier_mud_golem_rock_destroy"),
    (
        "SwiftnessAura",
        "modifier_centaur_khan_endurance_aura_bonus",
    ),
    ("DeathThroeRush", "modifier_furbolg_enrage_attack_speed"),
    (
        "ThunderClap",
        "modifier_polar_furbolg_ursa_warrior_thunder_clap",
    ),
    ("DeathThroePower", "modifier_furbolg_enrage_damage"),
    (
        "ToughnessAura",
        "modifier_enraged_wildkin_toughness_aura_bonus",
    ),
    (
        "MagicAmplificationAura",
        "modifier_black_drake_magic_amplification",
    ),
    (
        "DragonhideAura",
        "modifier_black_dragon_dragonhide_aura_bonus",
    ),
    ("WeakeningAura", "modifier_ancient_rock_golem_weakening"),
    ("GraniteAura", "modifier_granite_golem_hp_aura_bonus"),
    ("WarDrumsAura", "modifier_big_thunder_lizard_wardrums_aura"),
    ("Slam", "modifier_big_thunder_lizard_slam"),
    ("Frenzy", "modifier_big_thunder_lizard_frenzy"),
    (
        "TimeWarpAura",
        "modifier_frostbitten_golem_time_warp_aura_bonus",
    ),
    ("ProwlerAura", "modifier_spawnlord_aura_bonus"),
    ("Desecrate", "modifier_spawnlord_master_stomp"),
    ("Petrify", "modifier_spawnlord_master_freeze_root"),
    ("RiverbornAura", "modifier_frogmen_riverborn_aura_bonus"),
    ("Rally", "modifier_hill_troll_rally"),
    ("Tempest", "modifier_tornado_tempest_debuff"),
    ("WaterBubble", "modifier_frogmen_water_bubble"),
    ("Purge", "modifier_satyr_trickster_purge"),
    ("Fortified", "modifier_fountain_glyph"),
    ("MeatShield", "modifier_pudge_flesh_heap_block"),
    ("Dismembered", "modifier_pudge_dismember"),
    ("BackdoorProtection", "modifier_backdoor_protection_active"),
    ("Illusion", "modifier_illusion"),
    ("TimedLife", "modifier_kill"),
    ("NeutralUpgrade", "modifier_neutral_upgrade"),
    ("Flying", "modifier_courier_flying"),
    ("FountainInvulnerable", "modifier_fountain_invulnerability"),
    ("FountainPassive", "modifier_fountain_passive"),
    (
        "FountainDamage",
        "modifier_fountain_fury_swipes_damage_increase",
    ),
    ("VisionGrant", "modifier_lamp_on"),
    ("LampOff", "modifier_lamp_off"),
    ("CaptureGuard", "modifier_watch_tower_invulnerable"),
    ("ShrineReady", "bota_shrine_ready"),
    ("RuneHaste", "modifier_rune_haste"),
    ("RuneAmplifyDamage", "modifier_rune_doubledamage"),
    ("RuneArcane", "modifier_rune_arcane"),
    ("RuneRegeneration", "modifier_rune_regen"),
    ("RuneInvisibility", "modifier_rune_invis"),
    ("RuneShield", "modifier_rune_shield"),
    ("BottleRegeneration", "modifier_bottle_regeneration"),
    ("Corruption", "modifier_blight_stone_buff"),
    ("PoisonAttack", "modifier_item_orb_of_venom_slow"),
    ("Frost", "modifier_orb_of_frost_debuff"),
    ("Recovery", "modifier_aegis_regen"),
    ("Devotion", "modifier_roshan_devotion"),
    ("StrengthOfTheImmortal", "modifier_roshan_inherent_buffs"),
    ("DevotionAura", "modifier_roshan_devotion_aura"),
    ("SpellBlock", "modifier_roshan_spell_block"),
    ("RoshanSlam", "modifier_roshan_slam"),
    ("Bashed", "modifier_bashed"),
    ("RoarWindup", "modifier_roshan_revengeroar_windup"),
    ("RoarAura", "modifier_roshan_revengeroar_aura"),
    ("RoarDisarm", "modifier_roshan_revengeroar_disarm"),
    ("RoarOfRetribution", "modifier_roshan_revengeroar"),
    ("Silenced", "modifier_silence"),
    ("Grabbed", "modifier_roshan_grab"),
    ("Thrown", "modifier_roshan_grab_thrown"),
    ("Moving", "modifier_roshan_moving"),
    ("Knockback", "modifier_knockback"),
    (
        "ContactCooldown",
        "modifier_roshan_moving_knockback_timeout",
    ),
    ("Ethereal", "modifier_ghost_state"),
    ("TwinGateActive", "modifier_twin_gate_fx"),
    (
        "HookFollowthrough",
        "modifier_pudge_meat_hook_followthrough",
    ),
];

/// A value's variant name, as its `Debug` spells it.
fn variant_key(value: &impl std::fmt::Debug) -> String {
    let variant: String = format!("{value:?}")
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    MODIFIER_KEYS
        .iter()
        .find(|(known, _)| *known == variant)
        .unwrap_or_else(|| panic!("unmapped modifier variant {variant}"))
        .1
        .to_owned()
}

#[test]
fn every_effect_is_shown_under_its_registered_id() {
    // The enemies in a rot wear the effect its owner does: Dota's one
    // modifier_pudge_rot. Hasted movement shows as the haste it is.
    let mut shown: Vec<(u16, String)> = every_modifier()
        .iter()
        .map(|kind| match kind {
            ModifierKind::Journey { .. }
            | ModifierKind::DelayedSpell { .. }
            | ModifierKind::AuraEmitter { .. }
            | ModifierKind::Control { .. }
            | ModifierKind::ItemBuff { .. }
            | ModifierKind::SpellImmunity => (
                kind.effect_id().0,
                effect_def(kind.effect_id())
                    .expect("registered")
                    .key
                    .to_owned(),
            ),
            ModifierKind::BonusArmor { effect, .. } => (
                effect.0,
                effect_def(*effect).expect("registered").key.to_owned(),
            ),
            ModifierKind::Rotting { .. } => (kind.effect_id().0, "modifier_pudge_rot".to_string()),
            ModifierKind::Haste { .. } => {
                (kind.effect_id().0, "modifier_courier_burst".to_string())
            }
            // What an item lays shows under the effect its row names.
            ModifierKind::Laid { .. } => (
                kind.effect_id().0,
                effect_def(kind.effect_id())
                    .expect("a laid effect is live")
                    .key
                    .to_string(),
            ),
            _ => (kind.effect_id().0, variant_key(kind)),
        })
        .collect();
    shown.push((EffectId::FORTIFIED.0, "modifier_fountain_glyph".to_string()));
    shown.push((EffectId::ILLUSION.0, "modifier_illusion".to_string()));
    for entry in NEUTRAL_ABILITIES {
        let Some(effect) = spell_effect(entry.ability) else {
            continue;
        };
        let def = effect_def(effect).expect("a spell's effect is live");
        shown.push((effect.0, def.key.to_string()));
    }
    for entry in NEUTRAL_ABILITIES {
        let mut ids = Vec::new();
        if let bota_core::Spell::Cast {
            act: bota_core::Act::Grab(spec),
            ..
        } = *entry.ability.spell()
        {
            ids.extend([spec.grabbed_effect, spec.thrown_effect]);
        }
        if let Some(policy) = bota_core::migration_of(entry.ability) {
            ids.extend([
                policy.effect,
                policy.contact.effect,
                policy.contact.timeout_effect,
            ]);
        }
        for id in ids {
            shown.push((
                id.0,
                effect_def(id).expect("movement effect").key.to_owned(),
            ));
        }
    }
    let registry: Vec<RegisteredId> = registered(IdSpace::Effect).collect();
    for (id, name) in &shown {
        let def = effect_def(EffectId(*id))
            .unwrap_or_else(|| panic!("effect {id} ({name}) is retired or not in the table"));
        assert_eq!(def.key, name, "effect {id} changed meaning");
        assert!(
            registry
                .iter()
                .any(|entry| entry.id == *id && !entry.retired),
            "effect {id} ({name}) is not live in registry/ids.txt"
        );
    }
    for entry in registry.iter().filter(|entry| !entry.retired) {
        assert!(
            shown.iter().any(|(id, _)| *id == entry.id),
            "effect {} {} is live but nothing shows it",
            entry.id,
            entry.name
        );
    }
}
