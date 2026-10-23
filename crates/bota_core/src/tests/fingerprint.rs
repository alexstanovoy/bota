//! The rules digest covers every source of the rules.

use std::path::Path;

use crate::fingerprint::SOURCES;

/// A file added to this crate and not to the digest would let a rules change
/// pass for none.
#[test]
fn the_rules_digest_covers_every_source_file() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    walk(&src, &src, &mut found);
    found.sort();
    let mut listed: Vec<String> = SOURCES.iter().map(|(path, _)| path.to_string()).collect();
    listed.sort();
    assert_eq!(
        listed, found,
        "list every source in fingerprint.rs's SOURCES"
    );
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("the crate's sources") {
        let path = entry.expect("a directory entry").path();
        let relative = path.strip_prefix(root).expect("under src");
        if relative == Path::new("tests") {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let name = relative.to_str().expect("a UTF-8 path");
            found.push(name.replace('\\', "/"));
        }
    }
}
