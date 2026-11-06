//! One hero's script, over `npc_dota_hero_base`'s defaults, read into what bota keeps of it:
//! its body's numbers, its abilities and its talents.

use heatel::keyvalues::Kv;

use crate::ability::{self, Ability};
use crate::dec::{self, Milli};
use crate::install::HeroSource;

/// What bota keeps of one hero.
#[derive(Clone, Debug, PartialEq)]
pub struct Hero {
    pub key: String,
    /// `AttributePrimary` without its `DOTA_ATTRIBUTE_` prefix.
    pub primary: &'static str,
    /// Each body number by its script key, in thousandths: see [`NUMBERS`].
    pub numbers: Vec<(&'static str, Milli)>,
    pub melee: bool,
    /// The abilities of the slots `Ability1` to `Ability9`, in slot order.
    pub abilities: Vec<Ability>,
    /// The talents in bota's slot order: each tier's left talent (the odd `AbilityN`) first.
    pub talents: Vec<String>,
    /// The generic talents' own blocks, from `npc_abilities.txt`.
    pub shared_talents: Vec<Ability>,
}

/// The body numbers read, each from the hero or else the base hero.
pub const NUMBERS: [&str; 23] = [
    "AttributeBaseStrength",
    "AttributeStrengthGain",
    "AttributeBaseAgility",
    "AttributeAgilityGain",
    "AttributeBaseIntelligence",
    "AttributeIntelligenceGain",
    "ArmorPhysical",
    "MagicalResistance",
    "AttackDamageMin",
    "AttackDamageMax",
    "AttackRate",
    "AttackAnimationPoint",
    "AttackRange",
    "AttackAcquisitionRange",
    "ProjectileSpeed",
    "StatusHealth",
    "StatusHealthRegen",
    "StatusMana",
    "StatusManaRegen",
    "MovementSpeed",
    "MovementTurnRate",
    "VisionDaytimeRange",
    "VisionNighttimeRange",
];

/// The `AttributePrimary`s bota models.
const PRIMARIES: [&str; 3] = ["AGILITY", "INTELLECT", "STRENGTH"];

/// The talent slots, `Ability10` to `Ability17`, and the first ability slots before them.
const TALENT_SLOTS: std::ops::RangeInclusive<u32> = 10..=17;
const ABILITY_SLOTS: std::ops::RangeInclusive<u32> = 1..=9;

/// What fills a slot Dota keeps empty.
const EMPTY_SLOT: &str = "generic_hidden";

/// Keys of a hero's block read here or by [`NUMBERS`].
const READ: [&str; 5] = [
    "AbilityDefinitions",
    "AttackCapabilities",
    "AttributePrimary",
    "Facets",
    "HeroID",
];

/// Keys known and left out: presentation, cosmetics, the bot's script and the draft modes.
const LEFT_OUT: [&str; 50] = [
    "AbilityDraftAbilities",
    "AbilityPreview",
    "Adjectives",
    "AttackSpeedActivityModifiers",
    "BanSound",
    "Bot",
    "BotImplemented",
    "BoundsHullName",
    "CMEnabled",
    "Complexity",
    "Enabled",
    "GameSoundsFile",
    "GibTintColor",
    "GibType",
    "HeroGlowColor",
    "HeroOrderID",
    "HeroPool1",
    "HeroPool2",
    "HeroSelectSoundEffect",
    "HeroUnlockOrder",
    "IdleExpression",
    "IdleSoundLoop",
    "ItemSlots",
    "LastHitChallengeRival",
    "LoadoutScale",
    "Model",
    "ModelScale",
    "MovementSpeedActivityModifiers",
    "NameAliases",
    "Persona",
    "PickSound",
    "ProjectileModel",
    "RenderablePortrait",
    "Role",
    "Rolelevels",
    "SimilarHeroes",
    "SoundSet",
    "SpectatorLoadoutScale",
    "Team",
    "VersusScale",
    "VoiceFile",
    "new_player_enable",
    "particle_folder",
    "party_hat_effects",
    "precache",
    "showcase_attachments",
    "workshop_guide_name",
    "AbilityTalentStart",
    "HUD",
    "ARDMDisabled",
];

/// Reads the hero `key` of `source`.
pub fn read(source: &HeroSource, key: &str) -> Result<Hero, String> {
    let fail = |why: String| format!("{key}: {why}");
    let hero = source.hero(key).ok_or_else(|| fail("not read".into()))?;
    let base = source.base();
    for (name, _) in hero.block().ok_or_else(|| fail("no block".into()))? {
        let slot = name
            .strip_prefix("Ability")
            .and_then(|n| n.parse::<u32>().ok());
        let known = READ.contains(&name.as_str())
            || LEFT_OUT.contains(&name.as_str())
            || NUMBERS.contains(&name.as_str())
            || slot.is_some_and(|n| ABILITY_SLOTS.contains(&n) || TALENT_SLOTS.contains(&n));
        if !known {
            return Err(fail(format!("unknown key {name:?}")));
        }
    }
    let text = |name: &str| -> Result<&str, String> {
        match hero.get(name).or_else(|| base.get(name)) {
            Some(Kv::Text(text)) => Ok(text.trim()),
            Some(Kv::Block(_)) => Err(fail(format!("{name} is a block"))),
            None => Err(fail(format!("no {name}, nor in the base hero"))),
        }
    };
    let numbers = NUMBERS
        .iter()
        .map(|name| {
            let number = dec::milli(text(name)?).map_err(|e| fail(format!("{name}: {e}")))?;
            Ok((*name, number))
        })
        .collect::<Result<_, String>>()?;
    let primary = text("AttributePrimary")?;
    let primary = PRIMARIES
        .iter()
        .find(|p| primary.strip_prefix("DOTA_ATTRIBUTE_") == Some(**p))
        .copied()
        .ok_or_else(|| fail(format!("AttributePrimary {primary:?}")))?;
    let melee = match text("AttackCapabilities")? {
        "DOTA_UNIT_CAP_MELEE_ATTACK" => true,
        "DOTA_UNIT_CAP_RANGED_ATTACK" => false,
        other => return Err(fail(format!("AttackCapabilities {other:?}"))),
    };
    let dropped = dropped_facets(hero).map_err(fail)?;
    let definitions = hero.get("AbilityDefinitions");
    let slot = |n: u32| match hero.get(&format!("Ability{n}")) {
        Some(Kv::Text(text)) if !text.trim().is_empty() => Some(text.trim().to_owned()),
        _ => None,
    };
    let mut abilities = Vec::new();
    for name in ABILITY_SLOTS
        .filter_map(slot)
        .filter(|name| name != EMPTY_SLOT)
    {
        let block = definitions
            .and_then(|defs| defs.get(&name))
            .ok_or_else(|| fail(format!("{name} has no AbilityDefinitions block")))?;
        abilities.push(ability::read(&name, block, &dropped)?);
    }
    let mut talents = Vec::new();
    for tier in 0..4 {
        for side in [1, 0] {
            let n = TALENT_SLOTS.start() + 2 * tier + side;
            talents.push(slot(n).ok_or_else(|| fail(format!("no talent in Ability{n}")))?);
        }
    }
    let shared_talents = talents
        .iter()
        .filter_map(|talent| source.shared_ability(talent).map(|block| (talent, block)))
        .map(|(talent, block)| ability::read(talent, block, &[]))
        .collect::<Result<_, _>>()?;
    Ok(Hero {
        key: key.to_owned(),
        primary,
        numbers,
        melee,
        abilities,
        talents,
        shared_talents,
    })
}

/// The facets the hero marks `Deprecated`: bota plays none, and a bonus of one of these is
/// left out. A live facet fails the import.
fn dropped_facets(hero: &Kv) -> Result<Vec<String>, String> {
    let Some(facets) = hero.get("Facets") else {
        return Ok(Vec::new());
    };
    let mut dropped = Vec::new();
    for (name, facet) in facets.block().ok_or("Facets is no block")? {
        match facet.get("Deprecated") {
            Some(Kv::Text(flag)) if flag.trim() == "true" || flag.trim() == "1" => {
                dropped.push(name.clone());
            }
            _ => return Err(format!("the live facet {name:?}: bota plays no facet")),
        }
    }
    Ok(dropped)
}
