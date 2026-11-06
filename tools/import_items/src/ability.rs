//! One ability's block of a hero script or of `npc_abilities.txt`, read into what bota keeps of
//! it. As with items, a key, a flag or a bonus this tool does not know is refused: a patch that
//! brings one shows up as a failure to look at, not as a number silently left out.

use heatel::keyvalues::Kv;

use crate::dec::{self, Milli};
use crate::name;

/// What bota keeps of one ability.
#[derive(Clone, Debug, PartialEq)]
pub struct Ability {
    pub key: String,
    /// `AbilityBehavior` flags without their `DOTA_ABILITY_BEHAVIOR_` prefix, in file order.
    pub behavior: Vec<&'static str>,
    pub kind: Kind,
    pub innate: bool,
    /// `MaxLevel`, or Dota's default for the kind.
    pub max_level: i64,
    /// Seconds in thousandths, or world units, a level.
    pub cast_point: Vec<Milli>,
    pub cast_range: Vec<Milli>,
    pub channel: Vec<Milli>,
    pub cooldown: Vec<Milli>,
    pub mana: Vec<Milli>,
    /// `AbilityUnitDamageType` without its `DAMAGE_TYPE_` prefix.
    pub damage: Option<&'static str>,
    /// `SpellImmunityType`: whether it reaches a debuff-immune enemy.
    pub pierces_immunity: Option<bool>,
    /// `SpellDispellableType` without its `SPELL_DISPELLABLE_` prefix.
    pub dispel: Option<&'static str>,
    /// `AbilityUnitTargetTeam` without its `DOTA_UNIT_TARGET_TEAM_` prefix.
    pub target_team: Option<&'static str>,
    /// `AbilityUnitTargetType` flags without their `DOTA_UNIT_TARGET_` prefix.
    pub target_types: Vec<&'static str>,
    /// `AbilityUnitTargetFlags` flags without their `DOTA_UNIT_TARGET_FLAG_` prefix.
    pub target_flags: Vec<&'static str>,
    pub values: Vec<Value>,
}

/// `AbilityType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Basic,
    Ultimate,
    /// `ABILITY_TYPE_ATTRIBUTES`: a talent.
    Talent,
}

/// One of `AbilityValues`.
#[derive(Clone, Debug, PartialEq)]
pub struct Value {
    pub name: String,
    /// One number a level; a value only an upgrade gives is zero.
    pub levels: Vec<Milli>,
    /// What each talent or upgrade makes of it, by its key, in file order.
    pub bonuses: Vec<(String, Bonus)>,
}

/// What a talent or an upgrade makes of a value.
#[derive(Clone, Debug, PartialEq)]
pub enum Bonus {
    Add(Vec<Milli>),
    Times(Milli),
    Set(Vec<Milli>),
    Percent(Milli),
}

/// The `AbilityBehavior` flags an ability may carry.
pub const BEHAVIORS: [&str; 15] = [
    "AOE",
    "CHANNELLED",
    "DIRECTIONAL",
    "DONT_RESUME_ATTACK",
    "FORCE_NO_INNATE_UI",
    "HIDDEN",
    "IGNORE_BACKSWING",
    "IGNORE_CHANNEL",
    "IMMEDIATE",
    "NO_TARGET",
    "OPTIONAL_UNIT_TARGET",
    "PASSIVE",
    "POINT",
    "TOGGLE",
    "UNIT_TARGET",
];

/// The `AbilityUnitDamageType`s.
const DAMAGE_TYPES: [&str; 3] = ["MAGICAL", "PHYSICAL", "PURE"];
/// The `SpellDispellableType`s.
const DISPELS: [&str; 3] = ["NO", "YES", "YES_STRONG"];
/// The `AbilityUnitTargetTeam`s.
const TEAMS: [&str; 3] = ["BOTH", "ENEMY", "FRIENDLY"];
/// The `AbilityUnitTargetType` flags.
pub const TARGET_TYPES: [&str; 6] = ["BASIC", "BUILDING", "COURIER", "CREEP", "HERO", "OTHER"];
/// The `AbilityUnitTargetFlags` an imported ability carries.
pub const TARGET_FLAGS: [&str; 1] = ["MAGIC_IMMUNE_ENEMIES"];

/// Keys read into [`Ability`].
const READ: [&str; 16] = [
    "AbilityBehavior",
    "AbilityCastPoint",
    "AbilityCastRange",
    "AbilityChannelTime",
    "AbilityCooldown",
    "AbilityManaCost",
    "AbilityType",
    "AbilityUnitDamageType",
    "AbilityUnitTargetFlags",
    "AbilityUnitTargetTeam",
    "AbilityUnitTargetType",
    "AbilityValues",
    "Innate",
    "MaxLevel",
    "SpellDispellableType",
    "SpellImmunityType",
];

/// Keys known and left out: presentation, and the flags that only say an upgrade exists (its
/// values carry what it does).
const LEFT_OUT: [&str; 6] = [
    "AbilityCastAnimation",
    "AbilityChannelAnimation",
    "AbilitySound",
    "FightRecapLevel",
    "HasScepterUpgrade",
    "HasShardUpgrade",
];

/// The level-list keys that may stand among the values instead of beside them.
const LEVEL_KEYS: [&str; 4] = [
    "AbilityCastRange",
    "AbilityChannelTime",
    "AbilityCooldown",
    "AbilityManaCost",
];

/// Keys of a value's own block beside its `value` and its bonuses: how the client shows it, or
/// what an area increase does to it, which bota does not model.
const VALUE_MODIFIERS: [&str; 2] = ["CalculateSpellDamageTooltip", "affected_by_aoe_increase"];

/// Reads the block of the ability `key`. A bonus of a facet in `dropped_facets` (one the hero no
/// longer has) is left out; any other facet's is refused, since bota plays no facet.
pub fn read(key: &str, block: &Kv, dropped_facets: &[String]) -> Result<Ability, String> {
    let key = name::checked(key)?;
    let fail = |why: String| format!("{key}: {why}");
    let items = block.block().ok_or_else(|| fail("no block".into()))?;
    for (name, _) in items {
        if !READ.contains(&name.as_str()) && !LEFT_OUT.contains(&name.as_str()) {
            return Err(fail(format!("unknown key {name:?}")));
        }
    }
    let text = |name: &str| match block.get(name) {
        None => Ok(None),
        Some(Kv::Text(text)) => Ok(Some(text.trim())),
        Some(Kv::Block(_)) => Err(fail(format!("{name} is a block"))),
    };
    let kind = match text("AbilityType")? {
        None | Some("ABILITY_TYPE_BASIC") => Kind::Basic,
        Some("ABILITY_TYPE_ULTIMATE") => Kind::Ultimate,
        Some("ABILITY_TYPE_ATTRIBUTES") => Kind::Talent,
        Some(other) => return Err(fail(format!("unknown AbilityType {other:?}"))),
    };
    let values = values(block, dropped_facets).map_err(fail)?;
    let levels = |name: &str| -> Result<Vec<Milli>, String> {
        let beside = text(name)?
            .map(|t| dec::levels(t).map_err(|e| fail(format!("{name}: {e}"))))
            .transpose()?;
        let among = values
            .iter()
            .find(|v| v.name == name)
            .map(|v| v.levels.clone());
        match (beside, among) {
            (Some(_), Some(_)) => Err(fail(format!("{name} stands beside and among the values"))),
            (Some(levels), None) | (None, Some(levels)) => Ok(levels),
            (None, None) => Ok(vec![0]),
        }
    };
    let one_of = |name: &str, prefix: &str, known: &[&'static str]| -> Result<_, String> {
        text(name)?
            .map(|t| flag(t, prefix, known).map_err(|e| fail(format!("{name}: {e}"))))
            .transpose()
    };
    let max_level = match text("MaxLevel")? {
        Some(t) => dec::whole(t).map_err(|e| fail(format!("MaxLevel: {e}")))?,
        None => match kind {
            Kind::Basic => 4,
            Kind::Ultimate => 3,
            Kind::Talent => 1,
        },
    };
    Ok(Ability {
        key: key.to_owned(),
        behavior: flags(
            text("AbilityBehavior")?.unwrap_or(""),
            "DOTA_ABILITY_BEHAVIOR_",
            &BEHAVIORS,
        )
        .map_err(|e| fail(format!("AbilityBehavior: {e}")))?,
        kind,
        innate: match text("Innate")? {
            None | Some("0") => false,
            Some("1") => true,
            Some(other) => return Err(fail(format!("Innate {other:?}"))),
        },
        max_level,
        cast_point: text("AbilityCastPoint")?
            .map_or(Ok(vec![0]), dec::levels)
            .map_err(|e| fail(format!("AbilityCastPoint: {e}")))?,
        cast_range: levels("AbilityCastRange")?,
        channel: levels("AbilityChannelTime")?,
        cooldown: levels("AbilityCooldown")?,
        mana: levels("AbilityManaCost")?,
        damage: one_of("AbilityUnitDamageType", "DAMAGE_TYPE_", &DAMAGE_TYPES)?,
        pierces_immunity: match text("SpellImmunityType")? {
            None => None,
            Some("SPELL_IMMUNITY_ENEMIES_YES") => Some(true),
            Some("SPELL_IMMUNITY_ENEMIES_NO") => Some(false),
            Some(other) => return Err(fail(format!("unknown SpellImmunityType {other:?}"))),
        },
        dispel: one_of("SpellDispellableType", "SPELL_DISPELLABLE_", &DISPELS)?,
        target_team: one_of("AbilityUnitTargetTeam", "DOTA_UNIT_TARGET_TEAM_", &TEAMS)?,
        target_types: flags(
            text("AbilityUnitTargetType")?.unwrap_or(""),
            "DOTA_UNIT_TARGET_",
            &TARGET_TYPES,
        )
        .map_err(|e| fail(format!("AbilityUnitTargetType: {e}")))?,
        target_flags: flags(
            text("AbilityUnitTargetFlags")?.unwrap_or(""),
            "DOTA_UNIT_TARGET_FLAG_",
            &TARGET_FLAGS,
        )
        .map_err(|e| fail(format!("AbilityUnitTargetFlags: {e}")))?,
        values: values
            .into_iter()
            .filter(|v| !LEVEL_KEYS.contains(&v.name.as_str()) || !v.bonuses.is_empty())
            .collect(),
    })
}

/// The one known flag `text` names, its `prefix` dropped.
fn flag(text: &str, prefix: &str, known: &[&'static str]) -> Result<&'static str, String> {
    let name = text.trim().strip_prefix(prefix).unwrap_or(text.trim());
    known
        .iter()
        .find(|k| **k == name)
        .copied()
        .ok_or(format!("unknown {text:?}"))
}

/// The flags of a `|`-joined list, each one known, in file order.
fn flags(text: &str, prefix: &str, known: &[&'static str]) -> Result<Vec<&'static str>, String> {
    text.split('|')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(|f| flag(f, prefix, known))
        .collect()
}

/// `AbilityValues`: each a number a level, written alone or as the `value` of a block, with
/// its bonuses.
fn values(block: &Kv, dropped_facets: &[String]) -> Result<Vec<Value>, String> {
    let Some(values) = block.get("AbilityValues") else {
        return Ok(Vec::new());
    };
    let values = values.block().ok_or("AbilityValues is no block")?;
    values
        .iter()
        .map(|(name, value)| read_value(name, value, dropped_facets))
        .collect()
}

fn read_value(name: &str, value: &Kv, dropped_facets: &[String]) -> Result<Value, String> {
    let name = name::checked(name)?;
    let fail = |why: String| format!("{name}: {why}");
    let inner = match value {
        Kv::Text(text) => {
            return Ok(Value {
                name: name.to_owned(),
                levels: dec::levels(text).map_err(fail)?,
                bonuses: Vec::new(),
            });
        }
        Kv::Block(_) => value.block().unwrap_or(&[]),
    };
    let mut levels = None;
    let mut bonuses = Vec::new();
    for (key, text) in inner {
        let Kv::Text(text) = text else {
            return Err(fail(format!("{key} is a block")));
        };
        if key == "value" {
            levels = Some(dec::levels(text).map_err(fail)?);
        } else if VALUE_MODIFIERS.contains(&key.as_str()) {
            continue;
        } else if let Some(facet) = key.strip_prefix("special_bonus_facet_") {
            if !dropped_facets.iter().any(|dropped| dropped == facet) {
                return Err(fail(format!("a bonus of the live facet {facet:?}")));
            }
        } else if key.starts_with("special_bonus_") {
            let key = name::checked(key).map_err(fail)?;
            bonuses.push((
                key.to_owned(),
                bonus(text).map_err(|e| fail(format!("{key}: {e}")))?,
            ));
        } else {
            return Err(fail(format!("unknown key {key:?}")));
        }
    }
    let levels = match levels {
        Some(levels) => levels,
        None if !bonuses.is_empty() => vec![0],
        None => return Err(fail("no value".into())),
    };
    Ok(Value {
        name: name.to_owned(),
        levels,
        bonuses,
    })
}

/// A bonus as a script writes it: `x1.5`, `=80 =120`, `+15%`, `+150`, `+-10` or a bare `-4`.
fn bonus(text: &str) -> Result<Bonus, String> {
    let text = text.trim();
    if let Some(factor) = text.strip_prefix('x') {
        return Ok(Bonus::Times(dec::milli(factor)?));
    }
    if text.starts_with('=') {
        let set = text
            .split_whitespace()
            .map(|level| {
                level
                    .strip_prefix('=')
                    .ok_or(format!("{text:?} mixes = with plain levels"))
                    .and_then(dec::milli)
            })
            .collect::<Result<_, _>>()?;
        return Ok(Bonus::Set(set));
    }
    let added = text.strip_prefix('+').unwrap_or(text);
    if let Some(percent) = added.strip_suffix('%') {
        return Ok(Bonus::Percent(dec::milli(percent)?));
    }
    Ok(Bonus::Add(dec::levels(added)?))
}

#[cfg(test)]
mod tests {
    use heatel::keyvalues::parse_kv;

    use super::*;

    fn ability(body: &str) -> Result<Ability, String> {
        let root = parse_kv(&format!("\"x\" {{ {body} }}")).expect("parses");
        read("x", root.get("x").expect("the block"), &["gone".to_owned()])
    }

    /// Every way a script writes a talent's or an upgrade's bonus reads as what it does.
    #[test]
    fn every_bonus_a_script_writes_reads_as_what_it_does() {
        let read = ability(
            r#""AbilityValues" {
                "damage" { "value" "150 220" "special_bonus_unique_x_7" "+150"
                           "special_bonus_facet_gone" "=80 =120" }
                "slow" { "value" "-14" "special_bonus_unique_x_4" "+-10" }
                "AbilityCooldown" { "value" "18 16" "special_bonus_unique_x_5" "-4" }
                "AbilityCastRange" { "value" "1300" "special_bonus_scepter" "+15%" }
                "block" { "value" "8" "special_bonus_unique_x_1" "x1.5" }
                "shard_only" { "special_bonus_shard" "20" }
            }"#,
        )
        .expect("a known ability");
        let bonus = |name: &str| {
            let value = read.values.iter().find(|v| v.name == name).expect(name);
            (value.levels.clone(), value.bonuses.clone())
        };
        let named = |key: &str, b: Bonus| vec![(key.to_owned(), b)];
        assert_eq!(
            bonus("damage"),
            (
                vec![150_000, 220_000],
                named("special_bonus_unique_x_7", Bonus::Add(vec![150_000]))
            ),
            "an addition, the gone facet's bonus left out"
        );
        assert_eq!(
            bonus("slow").1,
            named("special_bonus_unique_x_4", Bonus::Add(vec![-10_000]))
        );
        assert_eq!(
            read.cooldown,
            vec![18_000, 16_000],
            "a level key among the values"
        );
        assert_eq!(
            bonus("AbilityCooldown").1,
            named("special_bonus_unique_x_5", Bonus::Add(vec![-4_000])),
            "and kept there for its bonus"
        );
        assert_eq!(
            bonus("AbilityCastRange").1,
            named("special_bonus_scepter", Bonus::Percent(15_000))
        );
        assert_eq!(
            bonus("block").1,
            named("special_bonus_unique_x_1", Bonus::Times(1_500))
        );
        assert_eq!(bonus("shard_only").0, vec![0], "only an upgrade gives it");
    }

    /// A patch that brings a key, a flag or a bonus this tool does not know fails the import.
    #[test]
    fn an_unknown_key_flag_or_bonus_is_refused() {
        assert!(ability(r#""AbilityBehavior" "DOTA_ABILITY_BEHAVIOR_POINT""#).is_ok());
        let refused = [
            r#""AbilityFlavour" "1""#,
            r#""AbilityBehavior" "DOTA_ABILITY_BEHAVIOR_VECTOR_TARGETING""#,
            r#""AbilityUnitTargetType" "DOTA_UNIT_TARGET_TREE""#,
            r#""AbilityValues" { "range" { "value" "300" "RequiresFacet" "x" } }"#,
            r#""AbilityValues" { "range" { "value" "300" "special_bonus_facet_live" "+1" } }"#,
            r#""AbilityValues" { "range" { "value" "300" "special_bonus_x" "1e3" } }"#,
            r#""AbilityCooldown" "5" "AbilityValues" { "AbilityCooldown" "5" }"#,
            r#""AbilityValues" { "x\", \"" "5" }"#,
            r#""AbilityValues" { "range" { "value" "1" "special_bonus_}{" "+1" } }"#,
        ];
        for body in refused {
            assert!(ability(body).is_err(), "{body}");
        }
    }
}
