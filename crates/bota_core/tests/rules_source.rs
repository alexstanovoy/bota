#[path = "../rules_source.rs"]
mod rules_source;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn normalized(source: &str) -> String {
    rules_source::canonical("rules.rs", source)
        .expect("valid Rust")
        .0
}

#[test]
fn formatting_docs_and_test_only_declarations_leave_the_identity_unchanged() {
    let plain = "use a::{x, y}; fn f(x: u8) -> [u8; 2] { match x { 0 => [1, 2], _ => [x, 2] } }";
    let editorial = "//! docs\nuse a::{ y,x, }; /// function\nfn f(x:u8,)->[u8;2]{ match x{0=>{[1,2,]},_=>[x,2,],} }\n\
        #[cfg(all(test, feature = \"anything\"))] const TEST: u8 = 5;\n\
        #[cfg(test)] mod tests { #[test] fn t() { panic!(\"anything\"); } }";
    assert_eq!(normalized(plain), normalized(editorial));
    let lines = format!("{plain}\n");
    assert_eq!(normalized(&lines), normalized(&lines.replace('\n', "\r\n")));
}

#[test]
fn tuple_commas_and_macro_matching_punctuation_remain_significant() {
    assert_ne!(
        normalized("fn f() { let _ = (1,); }"),
        normalized("fn f() { let _ = (1); }")
    );
    assert_ne!(
        normalized("fn f() { m![1,]; }"),
        normalized("fn f() { m![1]; }")
    );
    assert_ne!(
        normalized("fn f() { m![a => b]; }"),
        normalized("fn f() { m![a = > b]; }")
    );
    assert_ne!(
        normalized("const X: &str = r#\"// a comment\"#;"),
        normalized("const X: &str = r#\"// a different comment\"#;")
    );
}

#[test]
fn docs_and_test_only_items_inside_macros_do_not_count_as_rules() {
    let plain = "macro_rules! f { () => { fn g() {} }; }";
    let editorial = "macro_rules! f { () => { /// docs\nfn g() {} #[cfg(test)] impl Test { fn f() { panic!(\"test\"); } } #[test] fn generated_test() { panic!(\"test\"); } }; }";
    assert_eq!(normalized(plain), normalized(editorial));
}

#[test]
fn macro_matchers_and_stringified_attributes_remain_significant() {
    let matcher = "macro_rules! f { (#[cfg(test)] x) => { 1 }; }";
    assert_ne!(
        normalized(matcher),
        normalized(&matcher.replace("x)", "y)"))
    );
    let expanded = "macro_rules! f { () => { stringify!(#[doc = \"first\"]); }; }";
    assert_ne!(
        normalized(expanded),
        normalized(&expanded.replace("first", "second"))
    );
    let argument = "const X: &str = stringify!(#[doc = \"first\"]);";
    assert_ne!(
        normalized(argument),
        normalized(&argument.replace("first", "second"))
    );
}

#[test]
fn nonliteral_item_identifiers_are_refused_instead_of_leaving_a_hash_gap() {
    let sources = Sources::new();
    sources.write(
        "crates/bota_proto/src/ids.rs",
        "struct ItemId(u16); impl ItemId { const BOOTS: ItemId = ItemId(1 + 1); }",
    );
    assert_eq!(
        sources.item_identifiers().err().as_deref(),
        Some("crates/bota_proto/src/ids.rs: item identifier BOOTS must be a literal ItemId(u16)")
    );
}

struct Sources(PathBuf);

impl Sources {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "rules-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for path in [
            "crates/bota_core/src",
            "crates/bota_proto/src",
            "crates/bota_server/src/game",
            "crates/bota_server/src/engine",
        ] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        let sources = Self(root);
        sources.write("crates/bota_core/src/lib.rs", "mod rules; mod data;");
        sources.write("crates/bota_core/src/rules.rs", "const RULE: u8 = 1;");
        sources.write(
            "crates/bota_core/src/data/mod.rs",
            "mod item_mechanics; pub(crate) use item_mechanics::ITEM_ROWS;",
        );
        sources.write("crates/bota_proto/src/lib.rs", "mod ids;");
        sources.write("crates/bota_proto/src/ids.rs", "");
        sources.write("crates/bota_server/src/game/mod.rs", "");
        sources.write("crates/bota_server/src/engine/mod.rs", "");
        sources
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn item_identifiers(&self) -> Result<String, String> {
        rules_source::item_identifiers(&self.0, &rules_source::identity(&self.0)?.modules)
    }

    fn identity(&self) -> u64 {
        rules_source::identity(&self.0).unwrap().digest
    }
}

#[test]
fn review_module_graph_ignores_plumbing_tests_and_unreferenced_sources() {
    let sources = Sources::new();
    let initial = sources.identity();
    for path in [
        "crates/bota_server/src/net/mod.rs",
        "crates/bota_server/src/chat.rs",
        "crates/bota_server/src/profile/mod.rs",
        "crates/bota_server/src/main.rs",
        "crates/bota_server/src/spectators.rs",
        "crates/bota_server/src/lobby.rs",
        "crates/bota_server/src/game/tests/extra.rs",
        "crates/bota_server/src/game/unreferenced.rs",
        "crates/bota_proto/src/codec.rs",
    ] {
        sources.write(path, "const UNRELATED: u8 = 1;");
        assert_eq!(
            sources.identity(),
            initial,
            "{path} is not a simulation module"
        );
    }
    sources.write("crates/bota_server/src/game/mod.rs", "mod decision;");
    sources.write(
        "crates/bota_server/src/game/decision.rs",
        "const RULE: u8 = 1;",
    );
    let bound = sources.identity();
    sources.write(
        "crates/bota_server/src/game/decision.rs",
        "const RULE: u8 = 2;",
    );
    assert_ne!(sources.identity(), bound);
}

#[test]
fn review_private_item_data_helpers_do_not_change_rules_but_exports_are_guarded() {
    let sources = Sources::new();
    let initial = sources.identity();
    sources.write(
        "crates/bota_core/src/data/item_mechanics.rs",
        "const fn new_item_helper() -> u8 { 1 }",
    );
    assert_eq!(sources.identity(), initial);
    sources.write(
        "crates/bota_core/src/data/mod.rs",
        "mod item_mechanics; pub(crate) use item_mechanics::{ITEM_ROWS, other_rule};",
    );
    let error = rules_source::identity(&sources.0)
        .err()
        .expect("the private data fence must hold");
    assert!(error.contains("ITEM_ROWS"), "{error}");
}

#[test]
fn private_item_data_rejects_implementation_and_linkage_escapes() {
    for (source, reason) in [
        ("impl Rule { fn decide() -> u8 { 2 } }", "impl"),
        (
            "const _: () = { impl Rule { fn decide() -> u8 { 2 } } };",
            "impl",
        ),
        ("macro_rules! escaped { () => { 2 }; }", "macro"),
        (
            "#[macro_export] macro_rules! escaped { () => { 2 }; }",
            "macro",
        ),
        ("unsafe extern \"C\" { fn imported(); }", "extern"),
        ("extern \"C\" fn exported() {}", "extern"),
        ("include!(\"elsewhere.rs\");", "macro"),
    ] {
        let sources = Sources::new();
        sources.write("crates/bota_core/src/data/item_mechanics.rs", source);
        let error = rules_source::identity(&sources.0)
            .err()
            .expect("item data cannot define global behavior");
        assert!(error.contains("item_mechanics.rs"), "{error}");
        assert!(error.contains(reason), "{error}");
    }
}

#[test]
fn all_item_data_sources_are_validated_and_watched_without_entering_the_hash() {
    let sources = Sources::new();
    let initial = sources.identity();
    let path = "crates/bota_core/src/data/extra.rs";
    sources.write(path, "const fn helper() -> u8 { 1 }");
    let identity = rules_source::identity(&sources.0).unwrap();
    assert!(identity.files.contains(path));
    assert!(!identity.modules.contains(path));
    assert_eq!(identity.digest, initial);
}

#[test]
fn review_production_path_attributes_are_refused_and_test_paths_are_ignored() {
    let sources = Sources::new();
    sources.write(
        "crates/bota_core/src/lib.rs",
        "#[path = \"outside.rs\"] mod rules;",
    );
    let error = rules_source::identity(&sources.0)
        .err()
        .expect("path attributes must not escape discovery");
    assert!(error.contains("#[path]"), "{error}");
    sources.write(
        "crates/bota_core/src/lib.rs",
        "mod rules; #[cfg(test)] #[path = \"outside.rs\"] mod tests;",
    );
    assert!(rules_source::identity(&sources.0).is_ok());
}

#[test]
fn review_qualified_includes_are_hashed() {
    let sources = Sources::new();
    sources.write(
        "crates/bota_core/src/rules.rs",
        "const MAP: &[u8] = std::include_bytes!(\"../map.bin\");",
    );
    sources.write("crates/bota_core/map.bin", "one");
    let initial = sources.identity();
    sources.write("crates/bota_core/map.bin", "two");
    assert_ne!(sources.identity(), initial);
}

#[test]
fn review_anonymous_constants_with_items_keep_their_rule_code() {
    let source = "const _: () = { impl Rule { fn decide() -> u8 { 1 } } };";
    assert_ne!(
        normalized(source),
        normalized(&source.replace("{ 1 }", "{ 2 }"))
    );
}

#[test]
fn review_item_identifier_bindings_are_found_at_any_ast_depth() {
    let sources = Sources::new();
    let source = "struct ItemId(u16); mod nested { impl super::ItemId { const A: Self = Self(1); } } const _: () = { impl ItemId { const B: Self = Self(2); } };";
    sources.write("crates/bota_proto/src/ids.rs", source);
    let initial = sources.item_identifiers().unwrap();
    sources.write(
        "crates/bota_proto/src/ids.rs",
        &source.replace("Self(1)", "Self(3)"),
    );
    assert_ne!(sources.item_identifiers().unwrap(), initial);
    sources.write(
        "crates/bota_proto/src/ids.rs",
        &source.replace("Self(2)", "Self(4)"),
    );
    assert_ne!(sources.item_identifiers().unwrap(), initial);
}

#[test]
fn review_cfg_test_macro_fragments_cannot_swallow_following_rule_siblings() {
    let source =
        "macro_rules! m { ($test:item) => { #[cfg(test)] $test fn decide() -> u8 { 1 } }; }";
    assert_ne!(
        normalized(source),
        normalized(&source.replace("{ 1 }", "{ 2 }"))
    );
}

#[test]
fn review_parse_failures_fall_back_to_crlf_normalized_raw_source() {
    let source = "fn broken( {\n";
    let unix =
        rules_source::canonical("broken.rs", source).expect("defer the real syntax error to rustc");
    let windows = rules_source::canonical("broken.rs", &source.replace('\n', "\r\n")).unwrap();
    assert_eq!(unix.0, windows.0);
}

#[test]
fn review_nested_names_do_not_inherit_root_module_exclusions() {
    let sources = Sources::new();
    let source = "mod rules; mod data; mod nested { mod text { fn decide() -> u8 { 1 } } }";
    sources.write("crates/bota_core/src/lib.rs", source);
    let initial = sources.identity();
    sources.write(
        "crates/bota_core/src/lib.rs",
        &source.replace("{ 1 }", "{ 2 }"),
    );
    assert_ne!(sources.identity(), initial);
}

#[test]
fn documentation_in_audited_rule_declaration_macros_is_not_a_rule() {
    for (path, source) in [
        (
            "crates/bota_server/src/game/world.rs",
            "world_tables! { /// first\n pub struct World; state {} tables {} derived {} scratch {} }",
        ),
        (
            "crates/bota_proto/src/match_rule.rs",
            "kind_set! { /// first\n SomeSet of SomeKind }",
        ),
    ] {
        assert_eq!(
            rules_source::canonical(path, source).unwrap().0,
            rules_source::canonical(path, &source.replace("first", "second"))
                .unwrap()
                .0
        );
    }
}

#[test]
fn review_io_errors_name_the_input_path() {
    let sources = Sources::new();
    sources.write(
        "crates/bota_core/src/rules.rs",
        "const X: &str = include_str!(\"missing.txt\");",
    );
    let error = rules_source::identity(&sources.0).err().unwrap();
    assert!(error.contains("missing.txt"), "{error}");
}

impl Drop for Sources {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove this test's sources");
    }
}

#[test]
fn new_deleted_server_sources_and_included_map_data_change_the_identity() {
    let sources = Sources::new();
    sources.write(
        "crates/bota_core/src/lib.rs",
        "mod rules; mod data; mod map;",
    );
    sources.write(
        "crates/bota_core/src/map.rs",
        "const MAP: &[u8] = include_bytes!(\"../map.bin\");",
    );
    sources.write("crates/bota_core/map.bin", "first map");
    assert!(
        rules_source::identity(&sources.0)
            .unwrap()
            .files
            .contains("crates/bota_core/map.bin")
    );
    let initial = sources.identity();
    sources.write("crates/bota_server/src/game/mod.rs", "mod new_system;");
    sources.write(
        "crates/bota_server/src/game/new_system.rs",
        "const RULE: u8 = 1;",
    );
    assert_ne!(sources.identity(), initial);
    sources.write("crates/bota_server/src/game/mod.rs", "");
    std::fs::remove_file(sources.0.join("crates/bota_server/src/game/new_system.rs")).unwrap();
    assert_eq!(sources.identity(), initial);
    sources.write("crates/bota_core/map.bin", "second map");
    assert_ne!(sources.identity(), initial);
}

#[test]
fn source_discovery_ignores_tests_and_unrelated_client_files() {
    let sources = Sources::new();
    sources.write("crates/bota_core/src/rules.rs", "const RULE: u8 = 1;");
    let initial = sources.identity();
    for path in [
        "crates/bota_server/src/tests.rs",
        "crates/bota_server/src/game/tests/new.rs",
        "crates/bota_client_core/src/lib.rs",
    ] {
        sources.write(path, "not even Rust");
    }
    assert_eq!(sources.identity(), initial);
}

#[test]
fn escaping_or_dynamic_includes_are_refused_before_reading_them() {
    let sources = Sources::new();
    sources.write(
        "crates/bota_core/src/rules.rs",
        "const X: &str = include_str!(\"../../../../outside\");",
    );
    assert_eq!(
        rules_source::identity(&sources.0).err().as_deref(),
        Some("crates/bota_core/src/rules.rs: rule include escapes the repository")
    );
    sources.write(
        "crates/bota_core/src/rules.rs",
        "const X: &str = include_str!(env!(\"SECRET\"));",
    );
    assert_eq!(
        rules_source::identity(&sources.0).err().as_deref(),
        Some(
            "crates/bota_core/src/rules.rs: rule includes must name a literal repository-relative path"
        )
    );
}

#[cfg(unix)]
#[test]
fn symlinked_rule_inputs_are_refused() {
    let sources = Sources::new();
    sources.write("crates/bota_server/src/game/mod.rs", "mod rules;");
    std::os::unix::fs::symlink(
        "/outside",
        sources.0.join("crates/bota_server/src/game/rules.rs"),
    )
    .unwrap();
    assert_eq!(
        rules_source::identity(&sources.0).err().as_deref(),
        Some("crates/bota_server/src/game/rules.rs: rule inputs must not be symlinks")
    );
}

#[test]
fn the_build_uses_the_same_identity_as_the_source_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(
        rules_source::identity(&root).unwrap().digest,
        bota_core::RULES_IDENTITY
    );
}

#[test]
fn the_real_simulation_tree_has_no_raw_source_fallbacks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let identity = rules_source::identity(&root).unwrap();
    assert!(
        identity.raw_sources.is_empty(),
        "update the pinned parser before merging unsupported syntax: {:?}",
        identity.raw_sources
    );
}

#[test]
fn parse_fallbacks_are_observable_to_the_source_gate() {
    let sources = Sources::new();
    sources.write("crates/bota_core/src/rules.rs", "pub const VALUE: u8 = ;");
    assert!(
        rules_source::identity(&sources.0)
            .unwrap()
            .raw_sources
            .contains("crates/bota_core/src/rules.rs")
    );
}

#[test]
fn wire_view_impls_cannot_hide_new_simulation_helpers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    rules_source::validate_wire_view(&root).unwrap();
    let sources = Sources::new();
    sources.write(
        "crates/bota_proto/src/view.rs",
        "struct ClockShift; impl ClockShift { fn decide() -> u8 { 1 } }",
    );
    assert!(
        rules_source::validate_wire_view(&sources.0)
            .unwrap_err()
            .contains("simulation helper")
    );
}

#[test]
fn compatibility_helpers_are_outside_the_simulation_graph() {
    let sources = Sources::new();
    sources.write(
        "crates/bota_core/src/lib.rs",
        "mod rules; mod data; mod fingerprint; mod text;",
    );
    let path = "crates/bota_core/src/fingerprint.rs";
    let source = "pub const RULES_IDENTITY: u64 = include!(concat!(env!(\"OUT_DIR\"), \"/rules_identity.rs\")); fn decide() -> u8 { 1 }";
    sources.write(path, source);
    let initial = sources.identity();
    sources.write(path, &source.replace("{ 1 }", "{ 2 }"));
    assert_eq!(sources.identity(), initial);
}

#[test]
fn identical_sources_have_a_fixed_digest_independent_of_their_directory() {
    let first = Sources::new();
    let other = Sources::new();
    for sources in [&first, &other] {
        sources.write("crates/bota_core/src/rules.rs", "const RULE: u8 = 1;");
    }
    assert_eq!(first.identity(), other.identity());
    // Explicit framing and byte order, independent of Rust's Hash or Debug implementations.
    assert_eq!(first.identity(), 0x2f4e_a2e6_4c11_1c58);
}

#[test]
fn banner_construction_policy_remains_in_the_rules_identity() {
    let sources = Sources::new();
    sources.write(
        "crates/bota_core/src/lib.rs",
        "mod rules; mod data; mod banner;",
    );
    let policy = include_str!("../src/banner.rs");
    let changed = policy.replace("collision: 18,", "collision: 19,");
    assert_ne!(
        changed, policy,
        "the production construction policy is present"
    );
    sources.write("crates/bota_core/src/banner.rs", policy);
    let initial = sources.identity();
    sources.write("crates/bota_core/src/banner.rs", &changed);
    assert_ne!(
        sources.identity(),
        initial,
        "a Banner rule change must move its identity"
    );
}

#[test]
fn read_only_ability_projection_does_not_change_the_simulation_identity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let candidate = rules_source::identity(&root).expect("real simulation sources");
    let control = Sources::new();
    for path in &candidate.files {
        let destination = control.0.join(path);
        std::fs::create_dir_all(destination.parent().expect("source parent"))
            .expect("control directory");
        std::fs::copy(root.join(path), destination).expect("copy authoritative rule input");
    }
    let path = "crates/bota_core/src/lib.rs";
    let library = std::fs::read_to_string(control.0.join(path)).expect("core declarations");
    let rules_only = library
        .lines()
        .filter(|line| {
            !matches!(
                line.trim(),
                "mod ability_read;" | "pub use ability_read::*;"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    control.write(path, &rules_only);
    let expected = control.identity();
    println!(
        "candidate rules identity={:#018x}; simulation-only control={expected:#018x}",
        candidate.digest
    );
    assert_eq!(
        candidate.digest, expected,
        "read-only ability projections must not change simulation identity"
    );
    assert!(
        !candidate
            .modules
            .contains("crates/bota_core/src/ability_read.rs")
    );
    assert!(
        candidate
            .modules
            .contains("crates/bota_core/src/roshan_data.rs"),
        "authoritative shared data remains in the rules graph"
    );
    control.write(
        "crates/bota_client_core/src/ability_read.rs",
        "fn read_only_projection() -> u8 { 1 }",
    );
    assert_eq!(
        control.identity(),
        expected,
        "client projections are outside the rules graph"
    );
    control.write(
        "crates/bota_client_core/src/ability_read.rs",
        "fn read_only_projection() -> u8 { 2 }",
    );
    assert_eq!(
        control.identity(),
        expected,
        "changing a projection must not change rules"
    );
    let rule_path = "crates/bota_core/src/rules.rs";
    let rules = std::fs::read_to_string(control.0.join(rule_path)).expect("actual rules");
    control.write(
        rule_path,
        &format!("{rules}\nconst RULE_VIEW_TEST_SENTINEL: u8 = 1;\n"),
    );
    assert_ne!(
        control.identity(),
        expected,
        "actual core rule declarations remain protected"
    );
}
