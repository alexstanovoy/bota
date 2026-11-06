mod rules_source;

use std::path::Path;

fn main() {
    let manifest =
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo supplies the manifest directory");
    let root = Path::new(&manifest)
        .join("../..")
        .canonicalize()
        .expect("the repository root");
    println!("cargo:rerun-if-changed=rules_source.rs");
    let identity = rules_source::identity(&root).expect("contained, valid rule sources");
    if !identity.raw_sources.is_empty() {
        println!(
            "cargo:warning={} raw-source fallbacks; the source contract must pass before merge",
            identity.raw_sources.len()
        );
    }
    for path in &identity.files {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    let output = std::env::var_os("OUT_DIR").expect("Cargo supplies the output directory");
    let output = Path::new(&output);
    let contents = format!("{:#018x}_u64\n", identity.digest);
    write_generated(&output.join("rules_identity.rs"), &contents);
    let identifiers = rules_source::item_identifiers(&root, &identity.modules)
        .expect("literal item identifier declarations");
    write_generated(&output.join("item_identity.rs"), &identifiers);
}

fn write_generated(output: &Path, contents: &str) {
    if output
        .symlink_metadata()
        .is_ok_and(|meta| !meta.file_type().is_file())
    {
        panic!("the generated rules identity must be a regular file");
    }
    if std::fs::read_to_string(output).ok().as_deref() != Some(contents) {
        std::fs::write(output, contents)
            .expect("write the rules identity in Cargo's output directory");
    }
}
