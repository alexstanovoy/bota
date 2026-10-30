//! `import_items`: writes `crates/bota_core/src/data/items.rs` from the `scripts/npc/items.txt`
//! of a local Dota 2 install, for every item the id registry names. A development tool, never a
//! build step: the table it writes is committed, holds numbers and Dota's internal names only,
//! and names the client build it was read from.
//!
//! ```text
//! import_items [--dota <"dota 2 beta" folder>] [--check] [--add <item key>]...
//! ```
//!
//! `--add` appends a registry line for each new item key first. `--check` writes nothing and
//! fails when the committed table holds other items than the install gives (the client build it
//! names aside).

mod dec;
mod emit;
mod install;
mod item;
mod overrides;
mod registry;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The bota checkout this tool belongs to.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The committed table.
fn table_path() -> PathBuf {
    repo().join("crates/bota_core/src/data/items.rs")
}

/// The id registry.
fn registry_path() -> PathBuf {
    repo().join("crates/bota_proto/registry/ids.txt")
}

struct Args {
    dota: Option<PathBuf>,
    check: bool,
    add: Vec<String>,
}

fn args() -> Result<Args, String> {
    let mut parsed = Args {
        dota: None,
        check: false,
        add: Vec::new(),
    };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--dota" => parsed.dota = Some(words.next().ok_or("--dota needs a folder")?.into()),
            "--check" => parsed.check = true,
            "--add" => parsed
                .add
                .push(words.next().ok_or("--add needs an item key")?),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(parsed)
}

fn run() -> Result<(), String> {
    let args = args()?;
    let source = install::read(args.dota.as_deref())?;
    let registry_path = registry_path();
    let mut registry =
        std::fs::read_to_string(&registry_path).map_err(|e| format!("{registry_path:?}: {e}"))?;
    if !args.add.is_empty() {
        if args.check {
            return Err("--check and --add exclude each other".into());
        }
        registry = registry::append(&registry, &args.add, source.items())?;
        std::fs::write(&registry_path, &registry).map_err(|e| format!("{registry_path:?}: {e}"))?;
    }
    let table = rustfmt(&emit::table(&source, &registry::items(&registry)?)?)?;
    let path = table_path();
    if args.check {
        let committed = std::fs::read_to_string(&path).map_err(|e| format!("{path:?}: {e}"))?;
        if !emit::same_items(&committed, &table) {
            return Err(format!(
                "{path:?} differs from what the install gives: run import_items"
            ));
        }
        return Ok(());
    }
    std::fs::write(&path, table).map_err(|e| format!("{path:?}: {e}"))
}

/// `source` as `cargo fmt` leaves it in this repository, so a formatting pass never changes the
/// committed table.
fn rustfmt(source: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .current_dir(repo())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("rustfmt: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("rustfmt: no stdin")?;
    stdin
        .write_all(source.as_bytes())
        .map_err(|e| format!("rustfmt: {e}"))?;
    drop(stdin);
    let out = child
        .wait_with_output()
        .map_err(|e| format!("rustfmt: {e}"))?;
    if !out.status.success() {
        return Err(format!("rustfmt failed on the table: {}", out.status));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("rustfmt: {e}"))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("import_items: {why}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    /// The committed table is what the install on this machine gives, the build it names aside:
    /// a new client build that changes an item fails here until the table is regenerated, or
    /// the 7.41f number is kept by an override.
    #[test]
    #[ignore = "needs a Dota 2 install"]
    fn the_committed_table_is_what_the_install_gives() {
        let source = crate::install::read(None).expect("an install");
        let registry = std::fs::read_to_string(crate::registry_path()).expect("the registry");
        let items = crate::registry::items(&registry).expect("a well-formed registry");
        let table = crate::emit::table(&source, &items).expect("an importable install");
        let table = crate::rustfmt(&table).expect("rustfmt");
        let committed = std::fs::read_to_string(crate::table_path()).expect("the committed table");
        assert!(
            crate::emit::same_items(&committed, &table),
            "the committed table is stale: run import_items"
        );
    }
}
