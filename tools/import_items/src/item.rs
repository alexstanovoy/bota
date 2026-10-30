//! One item's block of `items.txt`, read into what bota keeps of it. A key or a behaviour this
//! tool does not know is refused: a patch that brings one shows up as a failure to look at, not
//! as a number silently left out.

use heatel::keyvalues::Kv;

use crate::dec::{self, Milli};

/// What bota keeps of one item.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: String,
    pub cost: i64,
    /// `AbilityBehavior` flags without their `DOTA_ABILITY_BEHAVIOR_` prefix, in file order.
    pub behavior: Vec<&'static str>,
    pub cast_range: i64,
    /// Seconds, in thousandths.
    pub cast_point: Milli,
    pub channel: Milli,
    pub cooldown: Milli,
    pub mana: i64,
    pub health: i64,
    pub charges: i64,
    /// `ItemStackable`, with `ItemStackableMax` where it has one.
    pub stackable: bool,
    pub stack_max: Option<i64>,
    /// `ItemStockInitial`, `ItemStockMax` and `ItemStockTime` (thousandths of seconds).
    pub stock: Option<(i64, i64, Milli)>,
    pub purchasable: bool,
    /// A recipe's `ItemResult` and its `ItemRequirements`, each a list of item keys.
    pub recipe: Option<(String, Vec<Vec<String>>)>,
    /// `AbilityValues` by key, one number a level.
    pub values: Vec<(String, Vec<Milli>)>,
}

/// The `AbilityBehavior` flags an item may carry.
pub const BEHAVIORS: [&str; 17] = [
    "AOE",
    "CHANNELLED",
    "DIRECTIONAL",
    "DONT_CANCEL_CHANNEL",
    "DONT_PROC_OTHER_ABILITIES",
    "DONT_RESUME_ATTACK",
    "IGNORE_BACKSWING",
    "IGNORE_CHANNEL",
    "IMMEDIATE",
    "NO_TARGET",
    "OPTIONAL_UNIT_TARGET",
    "OVERSHOOT",
    "PASSIVE",
    "POINT",
    "ROOT_DISABLES",
    "SUPPRESS_ASSOCIATED_CONSUMABLE",
    "UNIT_TARGET",
];

/// Keys read into [`Item`].
const READ: [&str; 18] = [
    "AbilityBehavior",
    "AbilityCastPoint",
    "AbilityCastRange",
    "AbilityChannelTime",
    "AbilityCooldown",
    "AbilityHealthCost",
    "AbilityManaCost",
    "AbilityValues",
    "ItemCost",
    "ItemInitialCharges",
    "ItemPurchasable",
    "ItemRequirements",
    "ItemResult",
    "ItemStackable",
    "ItemStackableMax",
    "ItemStockInitial",
    "ItemStockMax",
    "ItemStockTime",
];

/// Keys known and left out: presentation, the shop's suggestions, and rules bota models
/// elsewhere or not yet (targets, sharing, dispels, the shared cooldown's name).
const LEFT_OUT: [&str; 29] = [
    "AbilitySharedCooldown",
    "AbilityUnitTargetFlags",
    "AbilityUnitTargetTeam",
    "AbilityUnitTargetType",
    "Effect",
    "IsObsolete",
    "IsTempestDoubleClonable",
    "ItemDeclarations",
    "ItemDisassembleRule",
    "ItemDisplayCharges",
    "ItemPermanent",
    "ItemQuality",
    "ItemRecipe",
    "ItemRequiresCharges",
    "ItemShareability",
    "ItemShopTags",
    "ItemSupport",
    "Model",
    "SecretShop",
    "ShouldNotSuggestMainGame",
    "ShowGiveIndicatorOnTargetCast",
    "SpeciallyBannedFromNeutralSlot",
    "SpellDispellableType",
    "SuggestEarlygame",
    "SuggestLategame",
    "SuggestPregame",
    "UIDropSound",
    "UIPickupSound",
    "WorldDropSound",
];

/// Keys of a value's own block beside its `value`: how other things scale it.
const VALUE_MODIFIERS: [&str; 1] = ["affected_by_aoe_increase"];

/// Reads the block of the item `key`.
pub fn read(key: &str, block: &Kv) -> Result<Item, String> {
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
    let whole = |name: &str, default: i64| -> Result<i64, String> {
        text(name)?.map_or(Ok(default), |t| {
            dec::whole(t).map_err(|e| fail(format!("{name}: {e}")))
        })
    };
    let seconds = |name: &str| -> Result<Milli, String> {
        text(name)?.map_or(Ok(0), |t| {
            dec::milli(t).map_err(|e| fail(format!("{name}: {e}")))
        })
    };
    let stock = match text("ItemStockMax")? {
        None => None,
        Some(_) => Some((
            whole("ItemStockInitial", 0)?,
            whole("ItemStockMax", 0)?,
            seconds("ItemStockTime")?,
        )),
    };
    let recipe = match text("ItemResult")? {
        None => None,
        Some(result) => Some((result.to_owned(), requirements(block).map_err(fail)?)),
    };
    Ok(Item {
        key: key.to_owned(),
        cost: whole("ItemCost", 0)?,
        behavior: behavior(text("AbilityBehavior")?.unwrap_or("")).map_err(fail)?,
        cast_range: whole("AbilityCastRange", 0)?,
        cast_point: seconds("AbilityCastPoint")?,
        channel: seconds("AbilityChannelTime")?,
        cooldown: seconds("AbilityCooldown")?,
        mana: whole("AbilityManaCost", 0)?,
        health: whole("AbilityHealthCost", 0)?,
        charges: whole("ItemInitialCharges", 0)?,
        stackable: whole("ItemStackable", 0)? == 1,
        stack_max: text("ItemStackableMax")?
            .map(dec::whole)
            .transpose()
            .map_err(fail)?,
        stock,
        purchasable: whole("ItemPurchasable", 1)? == 1,
        recipe,
        values: values(block).map_err(fail)?,
    })
}

/// The flags of an `AbilityBehavior`, each one known.
fn behavior(text: &str) -> Result<Vec<&'static str>, String> {
    text.split('|')
        .map(str::trim)
        .filter(|flag| !flag.is_empty())
        .map(|flag| {
            let name = flag.strip_prefix("DOTA_ABILITY_BEHAVIOR_").unwrap_or(flag);
            BEHAVIORS
                .iter()
                .find(|known| **known == name)
                .copied()
                .ok_or(format!("unknown behaviour {flag:?}"))
        })
        .collect()
}

/// A recipe's ways to build, in file order: `"item_boots*;item_gloves"` is two parts (the
/// star, which part's state the build keeps, is not modelled).
fn requirements(block: &Kv) -> Result<Vec<Vec<String>>, String> {
    let ways = block
        .get("ItemRequirements")
        .and_then(Kv::block)
        .ok_or("a recipe without ItemRequirements")?;
    ways.iter()
        .map(|(_, way)| match way {
            Kv::Text(parts) => Ok(parts
                .split(';')
                .map(|part| part.trim().trim_end_matches('*').to_owned())
                .collect()),
            Kv::Block(_) => Err("a requirement is a block".to_owned()),
        })
        .collect()
}

/// `AbilityValues`: each a number a level, written alone or as the `value` of a block.
fn values(block: &Kv) -> Result<Vec<(String, Vec<Milli>)>, String> {
    let Some(values) = block.get("AbilityValues") else {
        return Ok(Vec::new());
    };
    let values = values.block().ok_or("AbilityValues is no block")?;
    values
        .iter()
        .map(|(name, value)| {
            let text = match value {
                Kv::Text(text) => text.as_str(),
                Kv::Block(_) => {
                    for (inner, _) in value.block().unwrap_or(&[]) {
                        if inner != "value" && !VALUE_MODIFIERS.contains(&inner.as_str()) {
                            return Err(format!("{name}: unknown key {inner:?}"));
                        }
                    }
                    match value.get("value") {
                        Some(Kv::Text(text)) => text.as_str(),
                        _ => return Err(format!("{name}: no value")),
                    }
                }
            };
            Ok((
                name.clone(),
                dec::levels(text).map_err(|e| format!("{name}: {e}"))?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::install::Source;

    use super::read;

    /// A patch that brings a key or a behaviour this tool does not know fails the import.
    #[test]
    fn an_unknown_key_or_behaviour_is_refused() {
        let item = |body: &str| {
            let source = Source::new(
                "fixture".into(),
                &format!("\"DOTAAbilities\" {{ \"item_x\" {{ {body} }} }}"),
            )
            .expect("parses");
            read("item_x", source.items().get("item_x").expect("an item"))
        };
        assert!(item(r#""ItemCost" "50" "AbilityValues" { "range" { "value" "300" } }"#).is_ok());
        let refused = [
            r#""ItemCost" "50" "ItemFlavour" "1""#,
            r#""AbilityBehavior" "DOTA_ABILITY_BEHAVIOR_ATTACK""#,
            r#""AbilityValues" { "range" { "value" "300" "RequiresFacet" "x" } }"#,
            r#""AbilityCooldown" "1e3""#,
        ];
        for body in refused {
            assert!(item(body).is_err(), "{body}");
        }
    }
}
