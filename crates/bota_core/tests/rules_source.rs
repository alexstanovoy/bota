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
    assert_eq!(normalized(plain), normalized(&plain.replace('\n', "\r\n")));
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
fn only_item_rows_are_left_to_the_catalogue_not_their_conversion_code() {
    let path = "crates/bota_core/src/data/item_mechanics.rs";
    let first = "fn from(x: u8) -> u8 { x } const ITEM_ROWS: &[u8] = &[1];";
    let row = first.replace("&[1]", "&[1, 2]");
    let helper = first.replace("{ x }", "{ x + 1 }");
    let digest = |source: &str| rules_source::canonical(path, source).unwrap().0;
    assert_eq!(digest(first), digest(&row));
    assert_ne!(digest(first), digest(&helper));
}

#[test]
fn nonliteral_item_identifiers_are_refused_instead_of_leaving_a_hash_gap() {
    let sources = Sources::new();
    sources.write(
        "crates/bota_proto/src/ids.rs",
        "struct ItemId(u16); impl ItemId { const BOOTS: ItemId = ItemId(1 + 1); }",
    );
    assert_eq!(
        rules_source::item_identifiers(&sources.0).err().as_deref(),
        Some("item identifier BOOTS must be a literal ItemId(u16)")
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
        for path in rules_source::SOURCE_ROOTS {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        Self(root)
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn identity(&self) -> u64 {
        rules_source::identity(&self.0).unwrap().digest
    }
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
        "crates/bota_core/src/map.rs",
        "const MAP: &[u8] = include_bytes!(\"../map.bin\");",
    );
    sources.write("crates/bota_core/map.bin", "first map");
    assert!(
        rules_source::identity(&sources.0)
            .unwrap()
            .data_files
            .contains("crates/bota_core/map.bin")
    );
    let initial = sources.identity();
    sources.write(
        "crates/bota_server/src/new_system.rs",
        "const RULE: u8 = 1;",
    );
    assert_ne!(sources.identity(), initial);
    std::fs::remove_file(sources.0.join("crates/bota_server/src/new_system.rs")).unwrap();
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
        Some("rule include escapes the repository")
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
    std::os::unix::fs::symlink(
        "/outside",
        sources.0.join("crates/bota_server/src/rules.rs"),
    )
    .unwrap();
    assert_eq!(
        rules_source::identity(&sources.0).err().as_deref(),
        Some("crates/bota_server/src/rules.rs: rule inputs must not be symlinks")
    );
}

#[test]
fn the_build_uses_the_same_identity_as_the_source_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(
        rules_source::identity(&root).unwrap().digest,
        bota_core::RULES_VERSION
    );
}

#[test]
fn fingerprint_source_is_hashed_without_reading_the_generated_identity() {
    let sources = Sources::new();
    let path = "crates/bota_core/src/fingerprint.rs";
    let source = "pub const RULES_VERSION: u64 = include!(concat!(env!(\"OUT_DIR\"), \"/rules_identity.rs\")); fn decide() -> u8 { 1 }";
    sources.write(path, source);
    let initial = sources.identity();
    sources.write(path, &source.replace("{ 1 }", "{ 2 }"));
    assert_ne!(sources.identity(), initial);
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
    assert_eq!(first.identity(), 0x4cd0_a4bb_639c_a36d);
}
