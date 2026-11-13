//! bota holds no player text: definitions expose ids and internal keys, and what a
//! player reads (its name, its words) is the client's, from the player's install or its own table.

/// The item table's sources, as the build reads them.
const DATA: [(&str, &str); 2] = [
    ("data/items.rs", include_str!("../data/items.rs")),
    (
        "data/item_mechanics.rs",
        include_str!("../data/item_mechanics.rs"),
    ),
];

/// `source` without its comments and without the arguments of its `assert!` and `panic!` calls,
/// which speak to a developer whose build fails.
fn without_comments_and_assertions(source: &str) -> String {
    use syn::visit_mut::{self, VisitMut};
    struct Diagnostics;
    impl VisitMut for Diagnostics {
        fn visit_attribute_mut(&mut self, node: &mut syn::Attribute) {
            if node.path().is_ident("doc") {
                *node = syn::parse_quote!(#[doc = ""]);
            }
        }
        fn visit_macro_mut(&mut self, node: &mut syn::Macro) {
            if node.path.is_ident("assert") || node.path.is_ident("panic") {
                node.tokens = Default::default();
            } else {
                visit_mut::visit_macro_mut(self, node);
            }
        }
        fn visit_expr_method_call_mut(&mut self, node: &mut syn::ExprMethodCall) {
            // Keep the receiver: an expect message is diagnostic, its value is not.
            if node.method == "expect" {
                node.args.clear();
            }
            visit_mut::visit_expr_method_call_mut(self, node);
        }
    }
    let mut syntax: syn::File = syn::parse_file(source).unwrap_or_else(|_| {
        // Replay status is a block, while definition tables are whole modules.
        syn::parse_file(&format!("fn status() {source}")).expect("valid source fence input")
    });
    Diagnostics.visit_file_mut(&mut syntax);
    quote::quote!(#syntax).to_string()
}

/// Every string the item table holds is a Dota key or a number (`item_yasha`,
/// `movement_speed_percent_bonus`, `2.25`): no display name, recipe title or other words a player
/// would read, under whatever field. A row that names an item in English fails here; its name
/// belongs to the client's table, keyed by the item's key.
#[test]
fn the_item_table_holds_keys_and_numbers_and_no_player_text() {
    assert_no_player_text(&DATA);
    assert_no_player_text(&[("item.rs", include_str!("../item.rs"))]);
}

fn assert_no_player_text(files: &[(&str, &str)]) {
    for (file, source) in files {
        if let Ok(syntax) = syn::parse_file(source) {
            for item in syntax.items {
                if let syn::Item::Struct(definition) = item
                    && [
                        "HeroDef",
                        "AbilityDef",
                        "EffectDef",
                        "KindDef",
                        "NeutralDef",
                        "ItemDef",
                    ]
                    .iter()
                    .any(|name| definition.ident == name)
                {
                    definition_fields_are_keys(file, &definition);
                }
            }
        }
        let code = without_comments_and_assertions(source);
        let mut rest = code.as_str();
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let close = after.find('"').expect("a closed string");
            let text = &after[..close];
            assert!(
                [
                    "AbilityCooldown",
                    "AbilityCastRange",
                    "AbilityCastPoint",
                    "AbilityManaCost"
                ]
                .contains(&text)
                    || text.bytes().all(|b| b.is_ascii_lowercase()
                        || b.is_ascii_digit()
                        || b"_.-".contains(&b)),
                "{file}: {text:?} is no Dota key or number"
            );
            rest = &after[close + 1..];
        }
    }
}

#[test]
fn the_hero_tables_hold_no_player_text() {
    assert_no_player_text(&[
        ("hero.rs", include_str!("../hero.rs")),
        ("talent.rs", include_str!("../talent.rs")),
        (
            "heroes/pudge/kit.rs",
            include_str!("../heroes/pudge/kit.rs"),
        ),
        (
            "heroes/shadow_fiend/kit.rs",
            include_str!("../heroes/shadow_fiend/kit.rs"),
        ),
    ]);
}

#[test]
fn the_ability_tables_hold_no_player_text() {
    assert_no_player_text(&[
        ("ability.rs", include_str!("../ability.rs")),
        ("ability_neutral.rs", include_str!("../ability_neutral.rs")),
        ("objective.rs", include_str!("../objective.rs")),
    ]);
}

#[test]
fn the_effect_tables_hold_no_player_text() {
    assert_no_player_text(&[("effect.rs", include_str!("../effect.rs"))]);
}

#[test]
fn the_unit_tables_hold_no_player_text() {
    assert_no_player_text(&[
        ("unit_kind.rs", include_str!("../unit_kind.rs")),
        ("ward.rs", include_str!("../ward.rs")),
        ("unit_neutral.rs", include_str!("../unit_neutral.rs")),
    ]);
}

#[test]
fn replay_status_exposes_facts_without_player_text() {
    let source = include_str!("../../../bota_client_core/src/ui/replay.rs");
    let file = syn::parse_file(source).expect("valid replay module");
    let block = file
        .items
        .iter()
        .find_map(|item| {
            let syn::Item::Impl(implementation) = item else {
                return None;
            };
            implementation.items.iter().find_map(|item| {
                let syn::ImplItem::Fn(method) = item else {
                    return None;
                };
                (method.sig.ident == "status").then_some(&method.block)
            })
        })
        .expect("the replay status method");
    assert!(
        !quote::quote!(#block).to_string().contains('"'),
        "replay status exposes facts; its words belong to clients"
    );
}

fn definition_fields_are_keys(file: &str, definition: &syn::ItemStruct) {
    use quote::ToTokens;
    let allowed = if definition.ident == "ItemDef" {
        ["key", "values", "wait_group"]
    } else {
        ["key", "unit", "npc"]
    };
    for field in &definition.fields {
        let name = field
            .ident
            .as_ref()
            .expect("named definition field")
            .to_string();
        assert!(
            !["name", "display_name", "label", "title", "description"].contains(&name.as_str()),
            "{file}: display field {name} belongs to clients"
        );
        let spelling = field.ty.to_token_stream().to_string();
        let holds_text = spelling
            .split(|character: char| !character.is_alphanumeric() && character != '_')
            .any(|word| matches!(word, "str" | "String"));
        if holds_text {
            assert!(
                allowed.contains(&name.as_str()),
                "{file}: string field {name} is not an internal key"
            );
        }
    }
}
