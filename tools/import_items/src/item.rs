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
    /// `ItemStockInitial`, `ItemStockMax`, `ItemStockTime` and `ItemInitialStockTime`
    /// (thousandths of seconds).
    pub stock: Option<(i64, i64, Milli, Milli)>,
    /// `ItemPurchasable`, unless `IsObsolete`: the game keeps an obsolete item's row and offers
    /// it nowhere, and so does bota, its rules kept to be offered again.
    pub purchasable: bool,
    pub secret_shop: bool,
    /// `SpellDispellableType` without its `SPELL_DISPELLABLE_` prefix: `NO`, `YES` or
    /// `YES_STRONG`; none where the item names none.
    pub dispel: Option<&'static str>,
    /// `SpellImmunityType` without its `SPELL_IMMUNITY_` prefix: `ENEMIES_NO` or `ENEMIES_YES`;
    /// none where the item names none.
    pub immunity: Option<&'static str>,
    pub droppable: bool,
    pub sellable: bool,
    pub killable: bool,
    pub backpack: bool,
    /// `ItemCanBeUsedWithoutInventory`: usable while it sits in the backpack.
    pub used_from_backpack: bool,
    pub cooldown_in_inventory: bool,
    /// A recipe's `ItemResult` and its `ItemRequirements`, each a list of item keys.
    pub recipe: Option<(String, Vec<Vec<String>>)>,
    /// `AbilityValues` by key, one number a level.
    pub values: Vec<(String, Vec<Milli>)>,
    /// The keys of the values `affected_by_aoe_increase` marks, in their order.
    pub aoe: Vec<String>,
    /// The keys of the values `apply_effectiveness` marks, in their order: those the Item
    /// Potency of the attack that carries them scales.
    pub potency: Vec<String>,
    /// `AbilitySharedCooldown`: the name of the cooldown it shares with every item that names
    /// the same one; none where it names none.
    pub shared_cooldown: Option<String>,
}

/// The `AbilityBehavior` flags an item may carry.
pub const BEHAVIORS: [&str; 19] = [
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
    "TOGGLE",
    "IGNORE_INVISIBLE",
];

/// Keys read into [`Item`].
const READ: &[&str] = &[
    "AbilityBehavior",
    "AbilityCastPoint",
    "AbilityCastRange",
    "AbilityChannelTime",
    "AbilityCooldown",
    "AbilityHealthCost",
    "AbilityManaCost",
    "AbilitySharedCooldown",
    "AbilityValues",
    "ItemCost",
    "ItemInitialCharges",
    "ItemInitialStockTime",
    "ItemPurchasable",
    "ItemDroppable",
    "ItemSellable",
    "ItemKillable",
    "AllowedInBackpack",
    "ItemCanBeUsedWithoutInventory",
    "IsObsolete",
    "CooldownPausedOutOfInventory",
    "ItemRequirements",
    "ItemResult",
    "ItemStackable",
    "ItemStackableMax",
    "ItemStockInitial",
    "ItemStockMax",
    "ItemStockTime",
    "SecretShop",
    "SpellDispellableType",
    "SpellImmunityType",
];

/// Keys known and left out: presentation, the shop's suggestions, and rules bota models
/// elsewhere or not yet (targets, sharing, dispels, Turbo's stock).
const LEFT_OUT: &[&str] = &[
    // The kind of a passive's damage: Skull Basher's bash damage rides with the attack, physical.
    "AbilityUnitDamageType",
    "AbilityUnitTargetFlags",
    "AbilityUnitTargetTeam",
    "AbilityUnitTargetType",
    "Effect",
    "FightRecapLevel",
    "IsTempestDoubleClonable",
    "ItemAllowCombineFromGround",
    // Alt-click notification metadata; it changes no simulation rule.
    "ItemAlertable",
    "ItemContributesToNetWorthWhenDropped",
    "ItemDeclarations",
    "ItemDisassembleRule",
    "ItemDisplayCharges",
    "ItemHideCharges",
    "ItemInitialStockTimeTurbo",
    // That a neutral camp's drop holds it: neutral item drops are not simulated yet.
    "ItemIsNeutralActiveDrop",
    "ItemPermanent",
    "ItemQuality",
    "ItemRecipe",
    "ItemRecipeConsumesCharges",
    "ItemRequiresCharges",
    "ItemShareability",
    "ItemShopTags",
    "ItemSupport",
    "Model",
    "ModelAlternate",
    "ShouldNotSuggestMainGame",
    "ShowGiveIndicatorOnTargetCast",
    "SpeciallyBannedFromNeutralSlot",
    "SuggestEarlygame",
    "SuggestLategame",
    "SuggestPregame",
    "UIDropSound",
    "UIPickupSound",
    "WorldDropSound",
];

/// The flag of a value's block that says an AoE bonus widens it.
const AOE_FLAG: &str = "affected_by_aoe_increase";
/// The flag of a value's block that says the attack's Item Potency scales it.
const POTENCY_FLAG: &str = "apply_effectiveness";
/// The flag of a value's block that says a neutral item's enchantment (Dormant Curio) raises it:
/// neutral items and their enchantments are not simulated yet.
const CURIO_FLAG: &str = "apply_curio_bonus";
/// Keys of a value's own block beside its `value`: how other things scale it.
const VALUE_MODIFIERS: [&str; 3] = [AOE_FLAG, POTENCY_FLAG, CURIO_FLAG];

/// The keys of the values whose block sets `flag` to 1, in their order.
fn marked(block: &Kv, flag: &str) -> Result<Vec<String>, String> {
    let Some(values) = block.get("AbilityValues").and_then(Kv::block) else {
        return Ok(Vec::new());
    };
    let mut keys = Vec::new();
    for (name, value) in values {
        match value.get(flag) {
            None => {}
            Some(Kv::Text(set)) if set.trim() == "1" => keys.push(name.clone()),
            Some(Kv::Text(set)) if set.trim() == "0" => {}
            Some(_) => return Err(format!("{name}: {flag} must be 0 or 1")),
        }
    }
    Ok(keys)
}

/// Reads the block of the item `key`.
pub fn read(key: &str, block: &Kv) -> Result<Item, String> {
    let key = crate::name::checked(key)?;
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
            seconds("ItemInitialStockTime")?,
        )),
    };
    let recipe = match text("ItemResult")? {
        None => None,
        Some(result) => Some((result.to_owned(), requirements(block).map_err(fail)?)),
    };
    let secret_shop = match whole("SecretShop", 0)? {
        0 => false,
        1 => true,
        _ => return Err(fail("SecretShop must be 0 or 1".to_owned())),
    };
    let one_of = |name: &str, prefix: &str, known: &[&'static str]| -> Result<_, String> {
        text(name)?
            .map(|value| {
                let bare = value.strip_prefix(prefix).unwrap_or(value);
                known
                    .iter()
                    .find(|k| **k == bare)
                    .copied()
                    .ok_or_else(|| fail(format!("unknown {name} {value:?}")))
            })
            .transpose()
    };
    let dispel = one_of(
        "SpellDispellableType",
        "SPELL_DISPELLABLE_",
        &["NO", "YES", "YES_STRONG"],
    )?;
    let immunity = one_of(
        "SpellImmunityType",
        "SPELL_IMMUNITY_",
        &["ENEMIES_NO", "ENEMIES_YES"],
    )?;
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
        purchasable: whole("ItemPurchasable", 1)? == 1 && whole("IsObsolete", 0)? == 0,
        secret_shop,
        dispel,
        immunity,
        droppable: whole("ItemDroppable", 1)? == 1,
        sellable: whole("ItemSellable", 1)? == 1,
        killable: whole("ItemKillable", 1)? == 1,
        backpack: whole("AllowedInBackpack", 1)? == 1,
        used_from_backpack: whole("ItemCanBeUsedWithoutInventory", 0)? == 1,
        cooldown_in_inventory: whole("CooldownPausedOutOfInventory", 0)? == 1,
        recipe,
        values: values(block).map_err(fail)?,
        aoe: marked(block, AOE_FLAG).map_err(fail)?,
        potency: marked(block, POTENCY_FLAG).map_err(fail)?,
        shared_cooldown: text("AbilitySharedCooldown")?.map(str::to_owned),
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
/// star, which part's state the build keeps, is not modelled). One semicolon may end the list
/// (7.41f's Octarine Core: `"item_soul_booster;item_tiara_of_selemene;"`); an empty list or an
/// empty part anywhere else is refused.
fn requirements(block: &Kv) -> Result<Vec<Vec<String>>, String> {
    let ways = block
        .get("ItemRequirements")
        .and_then(Kv::block)
        .ok_or("a recipe without ItemRequirements")?;
    ways.iter()
        .map(|(key, way)| match way {
            Kv::Text(parts) => parts_of(parts).map_err(|why| format!("requirement {key:?}: {why}")),
            Kv::Block(_) => Err("a requirement is a block".to_owned()),
        })
        .collect()
}

/// The parts one way to build names, its single terminal semicolon allowed.
fn parts_of(list: &str) -> Result<Vec<String>, String> {
    let list = list.trim();
    let list = list.strip_suffix(';').unwrap_or(list);
    let parts: Vec<String> = list
        .split(';')
        .map(|part| part.trim().trim_end_matches('*').to_owned())
        .collect();
    if parts.iter().any(String::is_empty) {
        return Err(format!("{list:?} names an empty part"));
    }
    Ok(parts)
}

/// `AbilityValues`: each a number a level, written alone or as the `value` of a block.
pub(crate) fn values(block: &Kv) -> Result<Vec<(String, Vec<Milli>)>, String> {
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
                crate::name::checked(name)?.to_owned(),
                dec::levels(text).map_err(|e| format!("{name}: {e}"))?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::install::Source;

    use super::{read, requirements};

    #[test]
    fn leading_interior_and_repeated_terminal_recipe_delimiters_are_refused() {
        for (parts, normalized) in [
            (";item_a", ";item_a"),
            ("item_a;;item_b", "item_a;;item_b"),
            ("item_a;;", "item_a;"),
            ("item_a; ;item_b", "item_a; ;item_b"),
        ] {
            let text = format!(
                r#""DOTAAbilities" {{ "item_recipe_x" {{ "ItemResult" "item_x" "ItemRequirements" {{ "01" "{parts}" }} }} }}"#
            );
            let source = Source::new("fixture".into(), &text).expect("KV");
            let error = read(
                "item_recipe_x",
                source.items().get("item_recipe_x").expect("recipe"),
            )
            .expect_err(parts);
            assert_eq!(
                error,
                format!("item_recipe_x: requirement \"01\": {normalized:?} names an empty part"),
                "{parts}"
            );
        }
    }

    #[test]
    fn an_empty_recipe_requirement_is_refused_instead_of_building_from_nothing() {
        let source = Source::new("fixture".into(), r#""DOTAAbilities" { "item_recipe_x" { "ItemResult" "item_x" "ItemRequirements" { "01" "; ; " } } }"#).expect("KV");
        let error = read(
            "item_recipe_x",
            source.items().get("item_recipe_x").expect("recipe"),
        )
        .expect_err("empty components");
        assert_eq!(
            error,
            "item_recipe_x: requirement \"01\": \"; \" names an empty part"
        );
    }

    #[test]
    fn a_trailing_recipe_separator_does_not_create_an_empty_component() {
        let source = Source::new("fixture".into(), r#""DOTAAbilities" { "item_recipe_x" { "ItemResult" "item_x" "ItemRequirements" { "01" "item_a;item_b;" } } }"#).expect("KV");
        let parsed = read(
            "item_recipe_x",
            source.items().get("item_recipe_x").expect("recipe"),
        )
        .expect("recipe data");
        assert_eq!(
            parsed.recipe.expect("build").1,
            vec![vec!["item_a".to_owned(), "item_b".to_owned()]]
        );
    }

    #[test]
    fn support_item_alert_metadata_does_not_change_simulation_data() {
        let parse = |body: &str| {
            let source = Source::new(
                "fixture".into(),
                &format!("\"DOTAAbilities\" {{ \"item_x\" {{ {body} }} }}"),
            )
            .expect("KV");
            read("item_x", source.items().get("item_x").expect("item"))
        };
        let alertable =
            parse(r#""ItemCost" "50" "ItemAlertable" "1""#).expect("support notification metadata");
        let plain = parse(r#""ItemCost" "50""#).expect("ordinary item");
        assert_eq!(alertable, plain);
    }

    #[test]
    fn secret_shop_flag_is_read_and_rejects_values_other_than_zero_or_one() {
        let parse = |value: &str| {
            let source = Source::new(
                "fixture".into(),
                &format!(r#""DOTAAbilities" {{ "item_x" {{ {value} }} }}"#),
            )
            .expect("KV");
            read("item_x", source.items().get("item_x").expect("item"))
        };
        assert!(!parse("").expect("default base shop").secret_shop);
        assert!(!parse(r#""SecretShop" "0""#).expect("base shop").secret_shop);
        assert!(
            parse(r#""SecretShop" "1""#)
                .expect("secret shop")
                .secret_shop
        );
        assert_eq!(
            parse(r#""SecretShop" "2""#).expect_err("invalid flag"),
            "item_x: SecretShop must be 0 or 1"
        );
    }

    /// An item the game marks `IsObsolete` keeps its row and every number, and nobody can buy
    /// it: Javelin in 7.42.
    #[test]
    fn an_obsolete_item_keeps_its_numbers_and_is_not_purchasable() {
        let parse = |value: &str| {
            let source = Source::new(
                "fixture".into(),
                &format!(
                    r#""DOTAAbilities" {{ "item_x" {{ "ItemCost" "900" {value} "AbilityValues" {{ "chance" "25" }} }} }}"#
                ),
            )
            .expect("KV");
            read("item_x", source.items().get("item_x").expect("item")).expect("an item")
        };
        assert!(parse("").purchasable);
        let obsolete = parse(r#""IsObsolete" "1""#);
        assert!(!obsolete.purchasable);
        assert_eq!(obsolete.cost, 900);
        assert_eq!(obsolete.values, vec![("chance".to_owned(), vec![25_000])]);
    }

    /// 7.42's item scripts mark the values an attack's Item Potency scales
    /// (`apply_effectiveness`) and an item used from the backpack
    /// (`ItemCanBeUsedWithoutInventory`): both are read; a neutral enchantment's mark
    /// (`apply_curio_bonus`) is known and left out.
    #[test]
    fn potency_marks_and_use_from_the_backpack_are_read() {
        let source = Source::new(
            "fixture".into(),
            r#""DOTAAbilities" { "item_x" { "ItemCanBeUsedWithoutInventory" "1" "AbilityValues" {
                "chance" { "value" "25" "apply_effectiveness" "1" }
                "damage" { "value" "60" "apply_curio_bonus" "1" }
                "range" { "value" "300" "apply_effectiveness" "0" } } } }"#,
        )
        .expect("KV");
        let item = read("item_x", source.items().get("item_x").expect("item")).expect("an item");
        assert_eq!(item.potency, vec!["chance".to_owned()]);
        assert!(item.used_from_backpack);
        assert_eq!(item.values.len(), 3);
    }

    /// A requirement list may end in one semicolon (7.41f's Octarine Core recipe:
    /// `"item_soul_booster;item_tiara_of_selemene;"`): no part is named after it. An empty part
    /// anywhere else, or an empty list, is refused by name rather than dropped.
    #[test]
    fn a_trailing_semicolon_names_no_part_and_an_empty_part_elsewhere_is_refused() {
        let parse = |list: &str| {
            let text = format!(
                r#""DOTAAbilities" {{ "item_recipe_x" {{ "ItemRequirements" {{ "01" "{list}" }} }} }}"#
            );
            let source = Source::new("fixture".into(), &text).expect("KV");
            requirements(source.items().get("item_recipe_x").expect("a recipe"))
        };
        let both = Ok(vec![vec!["item_a".to_owned(), "item_b".to_owned()]]);
        assert_eq!(parse("item_a;item_b*;"), both);
        assert_eq!(parse("item_a;item_b"), both);
        for (list, empty) in [
            ("item_a;;item_b", "item_a;;item_b"),
            (";item_a", ";item_a"),
            ("item_a;;", "item_a;"),
            ("", ""),
            (";", ""),
        ] {
            assert_eq!(
                parse(list),
                Err(format!(
                    r#"requirement "01": "{empty}" names an empty part"#
                )),
                "{list:?}"
            );
        }
    }

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
            r#""AbilityValues" { "x\"),(\"" "5" }"#,
        ];
        for body in refused {
            assert!(item(body).is_err(), "{body}");
        }
    }
}
