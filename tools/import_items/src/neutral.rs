//! The neutral creeps' unit rows and the abilities they carry, from the install's
//! `scripts/npc/npc_units.txt` and `npc_abilities.txt` (and the hero files that hold the
//! few abilities a neutral borrows), for every `neutral` line of the id registry but Roshan,
//! whose rows [`crate::boss`] writes.
//!
//! Nothing is skipped and nothing is defaulted without a word. A key this tool does not know, a
//! required key a script lacks, an empty number, a block where a number or a slot belongs, a key
//! written twice, more levels than a neutral ability has: each fails the import naming the unit or
//! ability and the key. What a script leaves out that bota still needs is an entry of
//! `neutral_overrides.rs` (a number with its source, or a default Dota gives), written into the
//! row; the readers in `bota_core` look every key up and add nothing.

use std::fmt::Write;
use std::path::Path;

use heatel::keyvalues::{Kv, parse_kv_repeats, text};
use heatel::{DirSource, Vpk, find_game_dir};

use crate::dec;
use crate::emit::decimal;
use crate::name;
use crate::neutral_overrides as overrides;

/// The unit of the boss, whose rows `boss.rs` writes.
const BOSS: &str = "npc_dota_roshan";

/// The most levels a neutral ability lists: bota's four.
const MAX_LEVELS: usize = 4;

/// Unit keys every unit must have, in the order the table lists them.
const REQUIRED: [&str; 11] = [
    "ArmorPhysical",
    "BountyGoldMax",
    "BountyGoldMin",
    "BountyXP",
    "MovementSpeed",
    "StatusHealth",
    "StatusHealthRegen",
    "StatusMana",
    "StatusManaRegen",
    "VisionDaytimeRange",
    "VisionNighttimeRange",
];

/// Unit keys a unit that attacks must have.
const ATTACKERS: [&str; 6] = [
    "AttackAcquisitionRange",
    "AttackAnimationPoint",
    "AttackDamageMax",
    "AttackDamageMin",
    "AttackRange",
    "AttackRate",
];

/// The missile speed a unit that attacks from afar must have.
const MISSILE_SPEED: &str = "ProjectileSpeed";

/// Unit keys a script may leave out, each with a default of `neutral_overrides.rs`.
const DEFAULTED: [&str; 4] = [
    "BaseAttackSpeed",
    "IsAncient",
    "MagicalResistance",
    "MovementTurnRate",
];

/// Unit keys known and left out: how it looks and sounds, whose side it is, or what bota's
/// neutral kinds have no use for (whether it is a summon, whether it is a neutral at all, its
/// `Level`, which bota does not model: a neutral's `base_level` stays 1).
const LEFT_OUT: [&str; 16] = [
    "AttackDamageType",
    "BaseClass",
    "HealthBarOffset",
    "IsNeutralUnitType",
    "IsSummoned",
    "Level",
    "MaterialGroup",
    "Model",
    "ModelScale",
    "ProjectileModel",
    "RingRadius",
    "SoundSet",
    "TeamName",
    "UnitRelationshipClass",
    "precache",
    "show_occluded_outline",
];

/// What a unit's `AttackCapabilities` may be.
const ATTACKS: [(&str, &str); 3] = [
    ("DOTA_UNIT_CAP_NO_ATTACK", "None"),
    ("DOTA_UNIT_CAP_MELEE_ATTACK", "Melee"),
    ("DOTA_UNIT_CAP_RANGED_ATTACK", "Ranged"),
];

/// What a unit's `MovementCapabilities` may be.
const MOVEMENTS: [(&str, &str); 2] = [
    ("DOTA_UNIT_CAP_MOVE_GROUND", "Ground"),
    ("DOTA_UNIT_CAP_MOVE_FLY", "Fly"),
];

/// What a unit's `BoundsHullName` may be.
const HULLS: [(&str, &str); 3] = [
    ("DOTA_HULL_SIZE_SMALL", "Small"),
    ("DOTA_HULL_SIZE_REGULAR", "Regular"),
    ("DOTA_HULL_SIZE_HERO", "Hero"),
];

/// The names of the keys that stand more than once in `items`.
fn repeated(items: &[(String, Kv)]) -> Option<&str> {
    items
        .iter()
        .enumerate()
        .find(|(at, (key, _))| items[..*at].iter().any(|(earlier, _)| earlier == key))
        .map(|(_, (key, _))| key.as_str())
}

/// The abilities a unit lists, in slot order, its empty slots left out.
fn slots<'a>(unit: &str, block: &'a Kv) -> Result<Vec<&'a str>, String> {
    let mut listed = Vec::new();
    for slot in 1..=8 {
        match block.get(&format!("Ability{slot}")) {
            Some(Kv::Text(key)) if !key.trim().is_empty() => listed.push(key.trim()),
            Some(Kv::Text(_)) | None => {}
            Some(Kv::Block(_)) => return Err(format!("{unit}: Ability{slot} is a block")),
        }
    }
    Ok(listed)
}

/// A replacement ability can share a script key with the old spell. Keep distinct catalogue keys.
fn ability_identity(script_key: &str) -> &str {
    match script_key {
        "kobold_disarm" => "bota_kobold_steal_weapon",
        "warpine_raider_seed_shot" => "bota_odobenus_snowball_shot",
        "black_dragon_fireball" => "bota_black_dragon_searing_swoop",
        _ => script_key,
    }
}

/// Whether `key` names one of a unit's ability slots.
fn is_slot(key: &str) -> bool {
    key.strip_prefix("Ability")
        .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8"))
}

/// One `neutral` line of the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Neutral {
    pub key: String,
    /// Whether the line is `live`: a `retired` neutral stays in the table, not offered.
    pub live: bool,
}

/// The `neutral` lines of the registry, in id order.
pub fn registered(registry: &str) -> Result<Vec<Neutral>, String> {
    registry
        .lines()
        .filter(|line| line.starts_with("neutral "))
        .map(|line| {
            let words: Vec<&str> = line.split(' ').collect();
            let [_, _, state, key] = words[..] else {
                return Err(format!("malformed registry line {line:?}"));
            };
            Ok(Neutral {
                key: name::checked(key)?.to_owned(),
                live: match state {
                    "live" => true,
                    "retired" => false,
                    _ => return Err(format!("malformed registry line {line:?}")),
                },
            })
        })
        .collect()
}

/// The ability `key` of a hero file's `AbilityDefinitions`, if the file holds it.
fn locate<'a>(root: &'a Kv, key: &str) -> Result<Option<&'a Kv>, String> {
    let mut found = None;
    for (hero, block) in root.get("DOTAHeroes").and_then(Kv::block).unwrap_or(&[]) {
        let Some(definitions) = block.get("AbilityDefinitions").and_then(Kv::block) else {
            continue;
        };
        if let Some(repeat) = repeated(definitions) {
            return Err(format!("{hero}: AbilityDefinitions holds {repeat} twice"));
        }
        if let Some((_, ability)) = definitions.iter().find(|(name, _)| name == key) {
            if found.is_some() {
                return Err(format!("{key} is defined by two heroes of one file"));
            }
            found = Some(ability);
        }
    }
    Ok(found)
}

/// Refuses a block that a script's text opens more than once, however its braces are laid out:
/// the parser merges such blocks, so the repeat would not show in what it reads. Counted from the
/// token stream: a key followed by an opening brace.
fn defined_once(file: &str, what: &str, key: &str) -> Result<(), String> {
    use heatel::keyvalues::{Token, tokens};
    let mut openings = 0;
    let mut before = None;
    for token in tokens(file) {
        if token == Token::Open && before == Some(Token::Text(key)) {
            openings += 1;
        }
        before = Some(token);
    }
    if openings > 1 {
        return Err(format!("{what}: {key} is defined {openings} times"));
    }
    Ok(())
}

/// The block of a retired unit the committed table holds, for a unit the install no longer has.
fn committed_unit(committed: &str, key: &str) -> Option<String> {
    let start = committed.find(&format!(
        "pub(crate) const {}: NeutralUnitData",
        key.to_uppercase()
    ))?;
    let rest = &committed[start..];
    let end = rest.find("\n};\n")? + "\n};\n".len();
    Some(rest[..end].replace("offered: true,", "offered: false,"))
}

/// The source of `crates/bota_core/src/neutral_data.rs`, with `committed` the table now in the
/// repository (the rows of a retired unit the install no longer has are carried over from it).
pub fn table(
    dota: Option<&Path>,
    registry: &str,
    committed: Option<&str>,
) -> Result<String, String> {
    let game = find_game_dir(dota).ok_or("no Dota 2 install found; pass --dota")?;
    let source = DirSource::new(&game);
    let package = Vpk::open(&source, "pak01_dir.vpk").map_err(|e| format!("pak01_dir.vpk: {e}"))?;
    let read = |path: &str| -> Result<String, String> {
        let bytes = package
            .read_file(&source, path)
            .map_err(|e| format!("{path}: {e}"))?;
        Ok(text(&bytes).map_err(|e| format!("{path}: {e}"))?.to_owned())
    };
    let units_text = read("scripts/npc/npc_units.txt")?;
    let abilities_text = read("scripts/npc/npc_abilities.txt")?;
    let units = parse_kv_repeats(&units_text).map_err(|e| e.to_string())?;
    let abilities = parse_kv_repeats(&abilities_text).map_err(|e| e.to_string())?;
    let units = units
        .get("DOTAUnits")
        .ok_or("npc_units.txt: no DOTAUnits block")?;
    let abilities = abilities
        .get("DOTAAbilities")
        .ok_or("npc_abilities.txt: no DOTAAbilities block")?;
    let carried: Vec<Neutral> = registered(registry)?
        .into_iter()
        .filter(|neutral| neutral.key != BOSS)
        .collect();
    let keys: Vec<String> = carried.iter().map(|neutral| neutral.key.clone()).collect();
    overrides::check_units(&keys)?;
    let mut out = header();
    let mut ability_keys: Vec<&str> = Vec::new();
    for neutral in &carried {
        let key = &neutral.key;
        match units.get(key) {
            Some(block) => {
                defined_once(&units_text, "npc_units.txt", key)?;
                write_unit(&mut out, key, block, neutral.live && !overrides::held(key))?;
                for slot in slots(key, block)? {
                    if !ability_keys.contains(&slot) {
                        ability_keys.push(slot);
                    }
                }
            }
            None if !neutral.live => {
                let kept = committed.and_then(|text| committed_unit(text, key)).ok_or(
                    format!(
                        "{key} is retired, gone from npc_units.txt, and the committed table has no row of it to keep"
                    ),
                )?;
                let _ = write!(out, "\n{kept}");
            }
            None => {
                return Err(format!(
                    "{key} is live in the registry but not in npc_units.txt"
                ));
            }
        }
    }
    let listed: Vec<&str> = units
        .block()
        .unwrap_or(&[])
        .iter()
        .map(|(key, _)| key.as_str())
        .filter(|key| key.starts_with("npc_dota_neutral_") && !keys.iter().any(|k| k == key))
        .collect();
    if !listed.is_empty() {
        eprintln!(
            "import_items: neutral units of the scripts that the registry does not carry: {}",
            listed.join(", ")
        );
    }
    let mut hero_files = Vec::new();
    for entry in package.entries() {
        if entry.dir() == "scripts/npc/heroes" && entry.ext() == "txt" {
            hero_files.push(entry.path());
        }
    }
    for key in &ability_keys {
        let mut borrowed = None;
        let block = match abilities.get(key) {
            Some(block) => {
                defined_once(&abilities_text, "npc_abilities.txt", key)?;
                block
            }
            None => {
                let mut roots = Vec::new();
                for path in &hero_files {
                    let file = read(path)?;
                    if file.contains(&format!("\"{key}\"")) {
                        roots.push(parse_kv_repeats(&file).map_err(|e| format!("{path}: {e}"))?);
                    }
                }
                let mut defining = Vec::new();
                for root in roots {
                    if locate(&root, key)?.is_some() {
                        defining.push(root);
                    }
                }
                if defining.len() != 1 {
                    return Err(format!(
                        "{key}: not in npc_abilities.txt, and {} hero files define it",
                        defining.len()
                    ));
                }
                let root = borrowed.insert(defining.remove(0));
                locate(root, key)?.ok_or("located above")?
            }
        };
        write_ability(&mut out, key, block)?;
    }
    for retired in overrides::RETIRED {
        if ability_keys
            .iter()
            .any(|key| ability_identity(key) == retired.ability)
        {
            return Err(format!(
                "{} is listed by a unit again: drop it from the retired abilities",
                retired.ability
            ));
        }
        write_retired(&mut out, retired)?;
    }
    overrides::check_abilities(&ability_keys)?;
    let _ = write!(
        out,
        "\n/// Every neutral unit row above, in the registry's order.\npub(crate) const UNIT_ROWS: &[&NeutralUnitData] = &[{}];\n\n/// Every neutral ability row above.\npub(crate) const ABILITY_ROWS: &[&ScriptData] = &[{}];\n",
        rows(keys.iter().map(String::as_str)),
        rows(ability_keys.iter().map(|key| ability_identity(key)))
            + &overrides::RETIRED
                .iter()
                .map(|retired| format!(", &{}", overrides::retired_constant(retired.ability)))
                .collect::<String>(),
    );
    Ok(out)
}

fn rows<'a>(keys: impl Iterator<Item = &'a str>) -> String {
    keys.map(|key| format!("&{}", key.to_uppercase()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn header() -> String {
    let mut out = String::from(
        "//! Neutral creep and ability numbers.\n\
         //! Regenerate with `cargo run --release --manifest-path tools/import_items/Cargo.toml -- --neutrals-only`.\n\
         //! Add `--check` to compare the generated table.\n",
    );
    out.push_str(
        "//!\n//! What the scripts do not hold, kept in `tools/import_items/src/neutral_overrides.rs`:\n",
    );
    out.push_str(&overrides::describe());
    out.push_str(&overrides::describe_retired());
    out.push_str(
        "\nuse crate::{Dec, NeutralUnitData, ScriptData, UnitAttack, UnitHull, UnitMovement};\n",
    );
    out
}

/// The word `text` is, as the Rust variant `known` pairs it with.
fn word<'a>(text: &str, known: &[(&str, &'a str)], what: &str) -> Result<&'a str, String> {
    known
        .iter()
        .find(|(script, _)| *script == text.trim())
        .map(|(_, variant)| *variant)
        .ok_or(format!("unknown {what} {text:?}"))
}

fn write_unit(out: &mut String, key: &str, block: &Kv, live: bool) -> Result<(), String> {
    let fail = |why: String| format!("{key}: {why}");
    let items = block.block().ok_or_else(|| fail("no block".into()))?;
    if let Some(repeat) = repeated(items) {
        return Err(fail(format!("{repeat} is written twice")));
    }
    for (name, _) in items {
        let known = REQUIRED.contains(&name.as_str())
            || ATTACKERS.contains(&name.as_str())
            || DEFAULTED.contains(&name.as_str())
            || name == MISSILE_SPEED
            || LEFT_OUT.contains(&name.as_str())
            || is_slot(name)
            || [
                "AttackCapabilities",
                "MovementCapabilities",
                "BoundsHullName",
            ]
            .contains(&name.as_str());
        if !known {
            return Err(fail(format!("unknown key {name:?}")));
        }
    }
    let text = |name: &str| match block.get(name) {
        None => Ok(None),
        Some(Kv::Text(text)) => Ok(Some(text.trim())),
        Some(Kv::Block(_)) => Err(fail(format!("{name} is a block"))),
    };
    let attack = word(
        text("AttackCapabilities")?.ok_or_else(|| fail("no AttackCapabilities".into()))?,
        &ATTACKS,
        "AttackCapabilities",
    )
    .map_err(fail)?;
    let movement = word(
        text("MovementCapabilities")?.ok_or_else(|| fail("no MovementCapabilities".into()))?,
        &MOVEMENTS,
        "MovementCapabilities",
    )
    .map_err(fail)?;
    let hull = match text("BoundsHullName")? {
        None => "None".to_owned(),
        Some(hull) => format!(
            "Some(UnitHull::{})",
            word(hull, &HULLS, "BoundsHullName").map_err(fail)?
        ),
    };
    let abilities: Vec<String> = slots(key, block)?
        .into_iter()
        .map(|slot| name::checked(slot).map(|slot| format!("{:?}", ability_identity(slot))))
        .collect::<Result<_, _>>()?;
    let _ = write!(
        out,
        "\npub(crate) const {name}: NeutralUnitData = NeutralUnitData {{\n    key: \"{key}\",\n    offered: {live},\n    attack: UnitAttack::{attack},\n    movement: UnitMovement::{movement},\n    hull: {hull},\n    abilities: &[{abilities}],\n    numbers: ScriptData {{ key: \"{key}\", values: &[\n",
        name = key.to_uppercase(),
        abilities = abilities.join(", "),
    );
    let number = |name: &str| -> Result<Vec<i64>, String> {
        let value = text(name)?.ok_or_else(|| fail(format!("no {name}")))?;
        if value.is_empty() {
            return Err(fail(format!("{name} is empty")));
        }
        let value = overrides::rounded(key, name, value).unwrap_or(value);
        dec::levels(value).map_err(|e| fail(format!("{name}: {e}")))
    };
    let attacks = attack != "None";
    let mut numbers: Vec<(&str, Vec<i64>)> = Vec::new();
    for name in REQUIRED {
        numbers.push((name, number(name)?));
    }
    for name in ATTACKERS {
        if attacks || text(name)?.is_some() {
            numbers.push((name, number(name)?));
        }
    }
    if attack == "Ranged" {
        numbers.push((MISSILE_SPEED, number(MISSILE_SPEED)?));
    } else if let Some(speed) = text(MISSILE_SPEED)?.filter(|speed| {
        !matches!(*speed, "" | "0") && !overrides::ignored(key, MISSILE_SPEED, speed)
    }) {
        return Err(fail(format!(
            "a unit that does not shoot names a {MISSILE_SPEED} of {speed:?}"
        )));
    }
    for name in DEFAULTED {
        if text(name)?.is_some() {
            numbers.push((name, number(name)?));
        } else {
            let default = overrides::UNIT_DEFAULTS
                .iter()
                .find(|default| default.name == name)
                .ok_or_else(|| fail(format!("{name} has no default in the override file")))?;
            numbers.push((name, dec::levels(default.value).map_err(fail)?));
        }
    }
    let extra = overrides::unit_numbers(key, attacks);
    for number in &extra {
        if numbers.iter().any(|(name, _)| *name == number.name) {
            return Err(fail(format!(
                "{}: the install holds it already: drop the override",
                number.name
            )));
        }
        numbers.push((number.name, dec::levels(number.value).map_err(fail)?));
    }
    if attacks && !extra.iter().any(|number| number.name == "AttackBackswing") {
        return Err(fail(
            "no AttackBackswing: add an entry to the override file (an unknown one says so)".into(),
        ));
    }
    for (name, levels) in numbers {
        let levels: Vec<String> = levels
            .iter()
            .map(|level| format!("Dec::of(\"{}\")", decimal(*level)))
            .collect();
        let _ = writeln!(out, "        (\"{name}\", &[{}]),", levels.join(", "));
    }
    out.push_str("    ] },\n};\n");
    Ok(())
}

/// Ability keys read as numbers.
const ABILITY_NUMBERS: [&str; 8] = [
    "AbilityCastPoint",
    "AbilityCastRange",
    "AbilityChannelTime",
    "AbilityCooldown",
    "AbilityDamage",
    "AbilityManaCost",
    "IsBreakable",
    "MaxLevel",
];

/// Ability keys known and left out: how it is aimed, what it counts as, how it looks and
/// sounds; bota's neutral abilities hold their own structure and read the numbers.
const ABILITY_LEFT_OUT: [&str; 16] = [
    "AbilityBehavior",
    "AbilityCastGestureSlot",
    "AbilityChannelAnimation",
    "AbilityTextureName",
    "AbilitySound",
    "AbilityType",
    "AbilityUnitDamageType",
    "AbilityUnitTargetFlags",
    "AbilityUnitTargetTeam",
    "AbilityUnitTargetType",
    "AbilityValues",
    "AnimationIgnoresModelScale",
    "FightRecapLevel",
    "Modelscale",
    "SpellDispellableType",
    "SpellImmunityType",
];

/// Keys of a value's own block beside its `value`: how the client shows it, or what an area
/// increase does to it.
const VALUE_MODIFIERS: [&str; 3] = [
    "CalculateSpellDamageTooltip",
    "DamageTypeTooltip",
    "affected_by_aoe_increase",
];

/// The levels of a number of an ability: one to [`MAX_LEVELS`].
fn levels(ability: &str, name: &str, value: &str) -> Result<Vec<i64>, String> {
    let fail = |why: String| format!("{ability}: {name}: {why}");
    let value = value.trim();
    if value.is_empty() {
        return Err(fail("is empty".into()));
    }
    let levels = dec::levels(value).map_err(fail)?;
    if levels.len() > MAX_LEVELS {
        return Err(fail(format!(
            "lists {} levels, and a neutral ability has {MAX_LEVELS}",
            levels.len()
        )));
    }
    Ok(levels)
}

/// Whether an ability is cast: its behavior is neither passive nor autocast (an autocast one is
/// an attack's rider, which the neutral AI never orders).
fn is_cast(ability: &str, block: &Kv) -> Result<bool, String> {
    match block.get("AbilityBehavior") {
        Some(Kv::Text(flags)) => Ok(!flags.split('|').any(|flag| {
            matches!(
                flag.trim(),
                "DOTA_ABILITY_BEHAVIOR_PASSIVE" | "DOTA_ABILITY_BEHAVIOR_AUTOCAST"
            )
        })),
        Some(Kv::Block(_)) => Err(format!("{ability}: AbilityBehavior is a block")),
        None => Err(format!("{ability}: no AbilityBehavior")),
    }
}

/// One ability's numbers: those beside its values, then its `AbilityValues`.
fn ability_numbers(key: &str, block: &Kv) -> Result<Vec<(String, Vec<i64>)>, String> {
    let fail = |why: String| format!("{key}: {why}");
    let items = block.block().ok_or_else(|| fail("no block".into()))?;
    if let Some(repeat) = repeated(items) {
        return Err(fail(format!("{repeat} is written twice")));
    }
    let mut numbers: Vec<(String, Vec<i64>)> = Vec::new();
    for (name, value) in items {
        if name == "AbilityCastAnimation" {
            let Kv::Text(activity) = value else {
                return Err(fail(format!("{name} is a block")));
            };
            let activity = activity.trim();
            if matches!(activity, "ACT_INVALID" | "ACT_DOTA_GENERIC_CHANNEL_1") {
                continue;
            }
            let slot = activity
                .strip_prefix("ACT_DOTA_CAST_ABILITY_")
                .and_then(|suffix| suffix.parse::<u8>().ok())
                .filter(|slot| (1..=7).contains(slot))
                .ok_or_else(|| fail(format!("unknown {name} {activity:?}")))?;
            numbers.push((
                "AbilityCastAnimationSlot".to_owned(),
                vec![i64::from(slot) * 1000],
            ));
            continue;
        }
        if ABILITY_LEFT_OUT.contains(&name.as_str()) {
            continue;
        }
        if !ABILITY_NUMBERS.contains(&name.as_str()) {
            return Err(fail(format!("unknown key {name:?}")));
        }
        let Kv::Text(text) = value else {
            return Err(fail(format!("{name} is a block")));
        };
        numbers.push((name.clone(), levels(key, name, text)?));
    }
    if let Some(values) = block.get("AbilityValues") {
        let values = values
            .block()
            .ok_or_else(|| fail("AbilityValues is no block".into()))?;
        if let Some(repeat) = repeated(values) {
            return Err(fail(format!("AbilityValues holds {repeat} twice")));
        }
        for (name, value) in values {
            let name = name::checked(name)?;
            let text = match value {
                Kv::Text(text) => text.as_str(),
                Kv::Block(_) => {
                    if let Some(repeat) = repeated(value.block().unwrap_or(&[])) {
                        return Err(fail(format!("{name}: {repeat} is written twice")));
                    }
                    for (inner, _) in value.block().unwrap_or(&[]) {
                        if inner != "value" && !VALUE_MODIFIERS.contains(&inner.as_str()) {
                            return Err(fail(format!("{name}: unknown key {inner:?}")));
                        }
                    }
                    match value.get("value") {
                        Some(Kv::Text(text)) => text.as_str(),
                        _ => return Err(fail(format!("{name}: no value"))),
                    }
                }
            };
            if numbers.iter().any(|(known, _)| known == name) {
                return Err(fail(format!("{name} stands beside and among the values")));
            }
            numbers.push((name.to_owned(), levels(key, name, text)?));
        }
    }
    Ok(numbers)
}

fn write_ability(out: &mut String, key: &str, block: &Kv) -> Result<(), String> {
    let mut numbers = ability_numbers(key, block)?;
    let cast = is_cast(key, block)?;
    let held = |numbers: &[(String, Vec<i64>)], name: &str| numbers.iter().any(|(n, _)| n == name);
    for number in overrides::ability_numbers(key) {
        if held(&numbers, number.name) {
            return Err(format!(
                "override of {key} {}: the install holds it already: drop the override",
                number.name
            ));
        }
        numbers.push((
            number.name.to_owned(),
            levels(key, number.name, number.levels)?,
        ));
    }
    for default in overrides::ABILITY_DEFAULTS {
        let applies = match default.which {
            overrides::Which::All => true,
            overrides::Which::Casts => cast,
        };
        if applies && !held(&numbers, default.name) {
            numbers.push((
                default.name.to_owned(),
                levels(key, default.name, default.value)?,
            ));
        }
    }
    if cast {
        for needed in ["AbilityManaCost", "AbilityCooldown"] {
            if !held(&numbers, needed) {
                return Err(format!("{key}: a cast without {needed}"));
            }
        }
        if !held(&numbers, "AbilityCastBackswing") {
            return Err(format!(
                "{key}: a cast without AbilityCastBackswing: add an entry to the override file (an unknown one says so)"
            ));
        }
    }
    write_row(out, ability_identity(key), &numbers)
}

/// The row of a retired ability, frozen at its last live values.
fn write_retired(out: &mut String, retired: &overrides::Retired) -> Result<(), String> {
    let mut numbers: Vec<(String, Vec<i64>)> = Vec::new();
    for (name, value) in retired.values {
        if numbers.iter().any(|(held, _)| held == name) {
            return Err(format!("{}: {name} is written twice", retired.ability));
        }
        numbers.push(((*name).to_owned(), levels(retired.ability, name, value)?));
    }
    let _ = writeln!(
        out,
        "\npub(crate) const {}: ScriptData = ScriptData {{\n    key: \"{}\",\n    values: &[",
        overrides::retired_constant(retired.ability),
        retired.ability
    );
    for (name, levels) in &numbers {
        let levels: Vec<String> = levels.iter().map(|level| format!("Dec({level})")).collect();
        let _ = writeln!(out, "        (\"{name}\", &[{}]),", levels.join(", "));
    }
    out.push_str("    ],\n};\n");
    Ok(())
}

/// The row of an ability: its numbers, level by level.
fn write_row(out: &mut String, key: &str, numbers: &[(String, Vec<i64>)]) -> Result<(), String> {
    let _ = writeln!(
        out,
        "\npub(crate) const {}: ScriptData = ScriptData {{\n    key: \"{key}\",\n    values: &[",
        key.to_uppercase()
    );
    for (name, levels) in numbers {
        let levels: Vec<String> = levels
            .iter()
            .map(|level| {
                i32::try_from(*level)
                    .map(|level| format!("Dec({level})"))
                    .map_err(|_| format!("{key}.{name}: number out of range"))
            })
            .collect::<Result<_, _>>()?;
        let _ = writeln!(out, "        (\"{name}\", &[{}]),", levels.join(", "));
    }
    out.push_str("    ],\n};\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED_KEYS: &str = r#""ArmorPhysical" "1" "BountyGoldMax" "5" "BountyGoldMin" "3"
        "BountyXP" "14" "MovementSpeed" "290" "StatusHealth" "240" "StatusHealthRegen" "0.5"
        "StatusMana" "0" "StatusManaRegen" "1" "VisionDaytimeRange" "800"
        "VisionNighttimeRange" "800""#;

    const ATTACK_KEYS: &str = r#""AttackAcquisitionRange" "500" "AttackAnimationPoint" "0.38"
        "AttackDamageMax" "16" "AttackDamageMin" "15" "AttackRange" "100" "AttackRate" "2""#;

    /// A melee unit with every key it must have, and an empty slot beside a filled one.
    fn melee() -> String {
        format!(
            r#""AttackCapabilities" "DOTA_UNIT_CAP_MELEE_ATTACK"
            "MovementCapabilities" "DOTA_UNIT_CAP_MOVE_GROUND" "Ability1" "a" "Ability2" ""
            {REQUIRED_KEYS} {ATTACK_KEYS}"#
        )
    }

    fn unit_of(body: &str, live: bool) -> Result<String, String> {
        let root = parse_kv_repeats(&format!("\"u\" {{ {body} }}")).expect("parses");
        let mut out = String::new();
        write_unit(&mut out, "u", root.get("u").expect("the block"), live)?;
        Ok(out)
    }

    fn unit(body: &str) -> Result<String, String> {
        unit_of(body, true)
    }

    /// A melee unit that names no hull has none, and a blank slot is no ability.
    #[test]
    fn a_unit_lists_its_filled_slots_and_names_no_hull_unless_the_script_does() {
        let out = unit(&melee()).expect("a plain unit");
        assert!(out.contains("hull: None,"), "{out}");
        assert!(out.contains("abilities: &[\"a\"],"), "{out}");
        assert!(out.contains("offered: true,"), "{out}");
    }

    /// A unit the registry retired stays in the table, marked not offered.
    #[test]
    fn a_retired_unit_is_written_not_offered() {
        let out = unit_of(&melee(), false).expect("a plain unit");
        assert!(out.contains("offered: false,"), "{out}");
    }

    /// A key this tool does not know fails the import, saying which.
    #[test]
    fn an_unknown_unit_key_is_refused_by_name() {
        let error = unit(&format!("{} \"WingSpan\" \"4\"", melee())).expect_err("a new key");
        assert_eq!(error, "u: unknown key \"WingSpan\"");
    }

    /// A hull this tool does not know fails the import, saying which.
    #[test]
    fn an_unknown_hull_is_refused_by_name() {
        let error = unit(&format!(
            "{} \"BoundsHullName\" \"DOTA_HULL_SIZE_HUGE\"",
            melee()
        ))
        .expect_err("a new hull");
        assert_eq!(error, "u: unknown BoundsHullName \"DOTA_HULL_SIZE_HUGE\"");
    }

    /// A key every unit must have, missing, fails the import naming the unit and the key.
    #[test]
    fn a_unit_without_a_required_key_is_refused_by_name() {
        let body = melee().replace("\"StatusHealth\" \"240\"", "");
        assert_eq!(unit(&body).expect_err("no health"), "u: no StatusHealth");
    }

    /// An empty number is no number: it is refused, not skipped.
    #[test]
    fn an_empty_number_is_refused_by_name() {
        let body = melee().replace("\"StatusHealth\" \"240\"", "\"StatusHealth\" \"\"");
        assert_eq!(unit(&body).expect_err("empty"), "u: StatusHealth is empty");
    }

    /// A key written twice would be read as its first: it is refused.
    #[test]
    fn a_key_written_twice_in_a_unit_is_refused_by_name() {
        let body = format!("{} \"StatusHealth\" \"999\"", melee());
        assert_eq!(
            unit(&body).expect_err("twice"),
            "u: StatusHealth is written twice"
        );
    }

    /// A block where an ability slot belongs is refused, not dropped.
    #[test]
    fn a_block_in_an_ability_slot_is_refused_by_name() {
        let body =
            format!("{} ", melee()).replace("\"Ability2\" \"\"", "\"Ability2\" { \"x\" \"1\" }");
        assert_eq!(unit(&body).expect_err("block"), "u: Ability2 is a block");
    }

    /// A unit that attacks must say how; one that does not need not.
    #[test]
    fn the_attack_numbers_are_required_of_an_attacker_alone() {
        let body = melee().replace("\"AttackRange\" \"100\"", "");
        assert_eq!(unit(&body).expect_err("no range"), "u: no AttackRange");
        let flier = format!(
            r#""AttackCapabilities" "DOTA_UNIT_CAP_NO_ATTACK"
            "MovementCapabilities" "DOTA_UNIT_CAP_MOVE_FLY" {REQUIRED_KEYS}"#
        );
        let out = unit(&flier).expect("a unit that does not attack");
        assert!(!out.contains("AttackRange"), "{out}");
        assert!(!out.contains("AttackBackswing"), "{out}");
    }

    /// A ranged attacker must name its missile speed, a melee one may write it empty.
    #[test]
    fn the_missile_speed_is_required_of_a_ranged_attacker_alone() {
        let ranged = melee().replace("DOTA_UNIT_CAP_MELEE_ATTACK", "DOTA_UNIT_CAP_RANGED_ATTACK");
        assert_eq!(
            unit(&ranged).expect_err("no missile"),
            "u: no ProjectileSpeed"
        );
        let out = unit(&format!("{ranged} \"ProjectileSpeed\" \"900\"")).expect("a ranged unit");
        assert!(
            out.contains("(\"ProjectileSpeed\", &[Dec::of(\"900\")])"),
            "{out}"
        );
        let out = unit(&format!("{} \"ProjectileSpeed\" \"\"", melee())).expect("a melee unit");
        assert!(!out.contains("ProjectileSpeed"), "{out}");
        assert_eq!(
            unit(&format!("{} \"ProjectileSpeed\" \"900\"", melee())).expect_err("a missile"),
            "u: a unit that does not shoot names a ProjectileSpeed of \"900\""
        );
    }

    /// A key a script leaves out and Dota defaults is written with the default of the override
    /// file; one it holds is written as it holds it.
    #[test]
    fn a_defaulted_key_is_written_with_its_default_or_its_own_value() {
        let out = unit(&melee()).expect("a plain unit");
        assert!(
            out.contains("(\"BaseAttackSpeed\", &[Dec::of(\"100\")])"),
            "{out}"
        );
        assert!(
            out.contains("(\"MovementTurnRate\", &[Dec::of(\"0.5\")])"),
            "{out}"
        );
        let own = unit(&format!("{} \"BaseAttackSpeed\" \"135\"", melee())).expect("a unit");
        assert!(
            own.contains("(\"BaseAttackSpeed\", &[Dec::of(\"135\")])"),
            "{own}"
        );
    }

    /// A number finer than a thousandth is refused, unless the override file rounds exactly that
    /// script text for that unit.
    #[test]
    fn a_number_finer_than_a_thousandth_is_refused_unless_the_override_file_rounds_it() {
        let seal = |point: &str| {
            let body = melee().replace(
                "\"AttackAnimationPoint\" \"0.38\"",
                &format!("\"AttackAnimationPoint\" \"{point}\""),
            );
            let root = parse_kv_repeats(&format!("\"u\" {{ {body} }}")).expect("parses");
            let mut out = String::new();
            write_unit(
                &mut out,
                "npc_dota_neutral_ogre_seal",
                root.get("u").expect("the block"),
                true,
            )
            .map(|()| out)
        };
        let rounded = seal("0.5714").expect("the seal's own 0.5714");
        assert!(
            rounded.contains("(\"AttackAnimationPoint\", &[Dec::of(\"0.571\")])"),
            "{rounded}"
        );
        assert!(seal("0.5715").is_err(), "any other text is refused");
    }

    /// A missile speed a melee unit's script holds is refused, unless the override file names that
    /// unit and that text as ignored.
    #[test]
    fn a_melee_units_missile_speed_is_ignored_only_where_the_override_file_says_so() {
        let tadpole = |speed: &str| {
            let body = format!("{} \"ProjectileSpeed\" \"{speed}\"", melee());
            let root = parse_kv_repeats(&format!("\"u\" {{ {body} }}")).expect("parses");
            let mut out = String::new();
            write_unit(
                &mut out,
                "npc_dota_neutral_tadpole",
                root.get("u").expect("the block"),
                true,
            )
            .map(|()| out)
        };
        assert!(
            !tadpole("1500")
                .expect("the listed text")
                .contains("ProjectileSpeed")
        );
        assert!(tadpole("1501").is_err(), "any other text is refused");
    }

    fn numbers_of(body: &str) -> Result<Vec<(String, Vec<i64>)>, String> {
        let root = parse_kv_repeats(&format!("\"a\" {{ {body} }}")).expect("parses");
        ability_numbers("a", root.get("a").expect("the block"))
    }

    /// Numbers beside the values and among them are read, levels and all; words are left out.
    #[test]
    fn an_ability_reads_the_numbers_beside_its_values_and_among_them() {
        let read = numbers_of(
            r#""AbilityCooldown" "10 9" "AbilityBehavior" "DOTA_ABILITY_BEHAVIOR_PASSIVE"
               "AbilityValues" { "damage" { "value" "5 6" "affected_by_aoe_increase" "1" } "radius" "300" }"#,
        )
        .expect("a known ability");
        assert_eq!(
            read,
            vec![
                ("AbilityCooldown".to_owned(), vec![10_000, 9_000]),
                ("damage".to_owned(), vec![5_000, 6_000]),
                ("radius".to_owned(), vec![300_000]),
            ]
        );
    }

    #[test]
    fn the_numbered_cast_animation_is_preserved() {
        assert_eq!(
            numbers_of(r#""AbilityCastAnimation" "ACT_DOTA_CAST_ABILITY_3""#)
                .expect("a numbered cast activity"),
            vec![("AbilityCastAnimationSlot".to_owned(), vec![3000])]
        );
    }

    #[test]
    fn an_unknown_numbered_cast_animation_is_refused_by_name() {
        assert_eq!(
            numbers_of(r#""AbilityCastAnimation" "ACT_DOTA_CAST_ABILITY_8""#)
                .expect_err("no such numbered cast activity"),
            "a: unknown AbilityCastAnimation \"ACT_DOTA_CAST_ABILITY_8\""
        );
    }

    /// A talent's or upgrade's bonus is something a neutral cannot have: it is refused rather
    /// than left out.
    #[test]
    fn a_bonus_on_a_neutral_ability_value_is_refused() {
        let error = numbers_of(
            r#""AbilityValues" { "damage" { "value" "5" "special_bonus_unique_x_1" "+5" } }"#,
        )
        .expect_err("a bonus");
        assert_eq!(error, "a: damage: unknown key \"special_bonus_unique_x_1\"");
    }

    /// An ability has four levels: a fifth number is refused, not dropped.
    #[test]
    fn a_fifth_level_of_an_ability_number_is_refused_by_name() {
        let error = numbers_of(r#""AbilityValues" { "damage" "1 2 3 4 5" }"#).expect_err("five");
        assert_eq!(
            error,
            "a: damage: lists 5 levels, and a neutral ability has 4"
        );
    }

    /// An empty number of an ability is refused, not skipped.
    #[test]
    fn an_empty_ability_number_is_refused_by_name() {
        let error = numbers_of(r#""AbilityValues" { "damage" "" }"#).expect_err("empty");
        assert_eq!(error, "a: damage: is empty");
    }

    /// A value written twice would be read as its first: it is refused.
    #[test]
    fn a_value_written_twice_in_an_ability_is_refused_by_name() {
        let error =
            numbers_of(r#""AbilityValues" { "damage" "1" "damage" "2" }"#).expect_err("twice");
        assert_eq!(error, "a: AbilityValues holds damage twice");
        let error =
            numbers_of(r#""AbilityCooldown" "1" "AbilityCooldown" "2""#).expect_err("twice");
        assert_eq!(error, "a: AbilityCooldown is written twice");
    }

    /// A unit or ability a script opens twice would be merged by the parser: it is refused.
    #[test]
    fn a_block_a_script_opens_twice_is_refused_by_name() {
        let file = "\t\"x\"\n\t{\n\t}\n\t\"x\"\n\t{\n\t}\n";
        assert_eq!(
            defined_once(file, "npc_units.txt", "x"),
            Err("npc_units.txt: x is defined 2 times".to_owned())
        );
        assert_eq!(defined_once(file, "npc_units.txt", "y"), Ok(()));
        let same_line = "\t\"x\" {\n\t}\n\t\"x\" { \"a\" \"1\" }\n";
        assert_eq!(
            defined_once(same_line, "npc_units.txt", "x"),
            Err("npc_units.txt: x is defined 2 times".to_owned()),
            "however the braces are laid out"
        );
        assert_eq!(
            defined_once("\"x\" \"y\"\n\"y\" { }\n", "npc_units.txt", "x"),
            Ok(()),
            "a value is no opening"
        );
    }

    /// A retired ability is written frozen at its last live values, from the override file.
    #[test]
    fn a_retired_ability_is_written_frozen_from_the_override_file() {
        let mut out = String::new();
        write_retired(&mut out, &overrides::RETIRED[0]).expect("a retired ability");
        assert!(
            out.contains("const RETIRED_BERSERKER_TROLL_BREAK: ScriptData"),
            "{out}"
        );
        assert!(out.contains("key: \"berserker_troll_break\""), "{out}");
        assert!(
            out.contains("Dec(10000), Dec(9000), Dec(8000), Dec(6000)"),
            "{out}"
        );
    }

    fn row_of(key: &str, body: &str) -> Result<String, String> {
        let root = parse_kv_repeats(&format!("\"a\" {{ {body} }}")).expect("parses");
        let mut out = String::new();
        write_ability(&mut out, key, root.get("a").expect("the block"))?;
        Ok(out)
    }

    const CAST: &str = r#""AbilityBehavior" "DOTA_ABILITY_BEHAVIOR_UNIT_TARGET"
        "AbilityManaCost" "75" "AbilityCooldown" "15""#;

    /// A cast whose backswing no entry of the override file gives is refused, naming it.
    #[test]
    fn a_cast_without_a_backswing_entry_is_refused_by_name() {
        let error = row_of("a", CAST).expect_err("no backswing");
        assert!(
            error.starts_with("a: a cast without AbilityCastBackswing"),
            "{error}"
        );
    }

    /// A cast must say what it costs and how long it waits.
    #[test]
    fn a_cast_without_mana_or_cooldown_is_refused_by_name() {
        let body = CAST.replace("\"AbilityManaCost\" \"75\"", "");
        assert_eq!(
            row_of("fel_beast_haunt", &body).expect_err("no mana"),
            "fel_beast_haunt: a cast without AbilityManaCost"
        );
    }

    /// What a cast leaves out has Dota's default, written into its row; a passive is no cast.
    #[test]
    fn a_cast_gets_its_defaults_written_and_a_passive_does_not() {
        let out = row_of("fel_beast_haunt", CAST).expect("a cast");
        assert!(out.contains("(\"AbilityCastPoint\", &[Dec(0)])"), "{out}");
        assert!(out.contains("(\"IsBreakable\", &[Dec(0)])"), "{out}");
        assert!(
            out.contains("(\"AbilityCastBackswing\", &[Dec(500)])"),
            "{out}"
        );
        let passive =
            row_of("a", r#""AbilityBehavior" "DOTA_ABILITY_BEHAVIOR_PASSIVE""#).expect("a passive");
        assert!(!passive.contains("AbilityCastPoint"), "{passive}");
        assert!(
            passive.contains("(\"IsBreakable\", &[Dec(0)])"),
            "{passive}"
        );
    }

    /// The backswing the default goes to every attacker no other entry names, and to no unit
    /// that does not attack.
    #[test]
    fn the_default_backswing_goes_to_attackers_no_entry_names() {
        let backswing = |unit: &str, attacks: bool| -> Vec<&str> {
            overrides::unit_numbers(unit, attacks)
                .iter()
                .map(|number| number.value)
                .collect()
        };
        assert_eq!(backswing("npc_dota_neutral_kobold", true), ["0.6"]);
        assert_eq!(backswing("npc_dota_neutral_ghost", true), ["0.3"]);
        assert!(backswing("npc_dota_enraged_wildkin_tornado", false).is_empty());
    }

    /// The registry's neutral lines, with their state; any other word is refused.
    #[test]
    fn the_registry_lists_its_neutral_units_with_their_state() {
        let registry = "hero 0 live npc_dota_hero_pudge\nneutral 0 live npc_dota_neutral_kobold\nneutral 1 retired npc_dota_roshan\n";
        let neutral = |key: &str, live| Neutral {
            key: key.to_owned(),
            live,
        };
        assert_eq!(
            registered(registry).expect("a registry"),
            [
                neutral("npc_dota_neutral_kobold", true),
                neutral("npc_dota_roshan", false)
            ]
        );
        assert_eq!(
            registered("neutral 0 dead npc_dota_neutral_kobold").expect_err("a state"),
            "malformed registry line \"neutral 0 dead npc_dota_neutral_kobold\""
        );
    }

    /// A retired unit the install no longer has is carried over from the committed table, its row
    /// marked not offered.
    #[test]
    fn a_retired_unit_gone_from_the_install_is_kept_from_the_committed_table() {
        let committed = "pub(crate) const NPC_DOTA_NEUTRAL_GONE: NeutralUnitData = NeutralUnitData {\n    key: \"npc_dota_neutral_gone\",\n    offered: true,\n};\n\npub(crate) const OTHER: u8 = 1;\n";
        let kept = committed_unit(committed, "npc_dota_neutral_gone").expect("the row");
        assert!(kept.contains("offered: false,"), "{kept}");
        assert!(!kept.contains("OTHER"), "{kept}");
        assert_eq!(committed_unit(committed, "npc_dota_neutral_none"), None);
    }

    #[test]
    #[ignore = "needs a Dota 2 install"]
    fn the_committed_neutral_table_is_what_the_install_gives() {
        let dota = std::env::var_os("BOTA_DOTA_PATH").map(std::path::PathBuf::from);
        let registry = std::fs::read_to_string(crate::registry_path()).expect("the registry");
        let committed =
            std::fs::read_to_string(crate::neutrals_path()).expect("the committed table");
        let table = crate::rustfmt(
            &table(dota.as_deref(), &registry, Some(&committed)).expect("an install"),
        )
        .expect("rustfmt");
        assert_eq!(
            committed, table,
            "the committed neutral table is stale: run import_items"
        );
    }
}
