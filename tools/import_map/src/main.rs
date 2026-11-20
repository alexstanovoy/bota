//! Regenerates landmarks, terrain, forests and vision blockers from the map packages.
//!
//! `--dota` selects an asset folder. Without it the tool reads `BOTA_MAP_SOURCE_PATH`,
//! then `BOTA_DOTA_PATH`, then the first available library. `--check` verifies the
//! package fingerprints and the generated tables without writing them.

mod emit;
mod entity;
mod forest;
mod frame;
mod ground;
mod install;
mod overrides;
mod places;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The bota checkout this tool belongs to.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The generated landmark table.
fn places_path() -> PathBuf {
    repo().join("crates/bota_core/src/map_data.rs")
}

/// The generated ground, forest and blocker table.
fn ground_path() -> PathBuf {
    repo().join("crates/bota_server/src/game/config/dota_map.rs")
}

/// The hero demo map's generated ground, forest and blocker table.
fn demo_ground_path() -> PathBuf {
    repo().join("crates/bota_server/src/game/config/demo_map.rs")
}

/// Generate every table with the same formatting as the workspace.
fn generate(dota: Option<&Path>) -> Result<Vec<(PathBuf, String)>, String> {
    let map = install::Map::open(dota, install::DOTA)?;
    let demo = install::Map::open(dota, install::HERO_DEMO)?;
    Ok(vec![
        (places_path(), rustfmt(&places::table(&map, &demo)?)?),
        (
            ground_path(),
            rustfmt(&ground(&map, &forest::DOTA_NAMES, overrides::DOTA_CELLS)?)?,
        ),
        (
            demo_ground_path(),
            rustfmt(&ground(&demo, &forest::DEMO_NAMES, overrides::DEMO_CELLS)?)?,
        ),
    ])
}

/// A map's generated ground, forest and blocker table.
fn ground(
    map: &install::Map,
    names: &forest::Names,
    cells: &[overrides::CellOverride],
) -> Result<String, String> {
    let cells = ground::cells(map, cells)?;
    Ok(forest::table(
        names,
        &map.fingerprint,
        &ground::runs(&cells, names.runs_by_row),
        &forest::trees(map, names.trees_by_text)?,
        &forest::blockers(map)?,
    ))
}

struct Args {
    dota: Option<PathBuf>,
    check: bool,
}

fn args() -> Result<Args, String> {
    let mut parsed = Args {
        dota: None,
        check: false,
    };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--dota" => parsed.dota = Some(words.next().ok_or("--dota needs a folder")?.into()),
            "--check" => parsed.check = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(parsed)
}

/// Verify the map package identity before decoding either input.
fn same_packages(dota: Option<&Path>) -> Result<(), String> {
    let game = install::game_dir(dota)?;
    let committed =
        std::fs::read_to_string(places_path()).map_err(|e| format!("{:?}: {e}", places_path()))?;
    let expected = emit::source_of(&committed).ok_or("the map table has no package fingerprint")?;
    let actual = format!(
        "{}/{}",
        install::fingerprint(&game, install::DOTA)?,
        install::fingerprint(&game, install::HERO_DEMO)?,
    );
    if expected != actual {
        return Err(format!(
            "map package mismatch: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args = args()?;
    if args.check {
        same_packages(args.dota.as_deref())?;
    }
    for (path, made) in generate(args.dota.as_deref())? {
        if args.check {
            let committed = std::fs::read_to_string(&path).map_err(|e| format!("{path:?}: {e}"))?;
            if !emit::same(&committed, &made) {
                return Err(format!(
                    "{path:?} differs from the package data: run import_map"
                ));
            }
            continue;
        }
        std::fs::write(&path, made).map_err(|e| format!("{path:?}: {e}"))?;
    }
    Ok(())
}

/// `source` as `cargo fmt` leaves it in this repository, so a formatting pass never changes a
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
        return Err(format!("rustfmt failed on a table: {}", out.status));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("rustfmt: {e}"))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("import_map: {why}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs the source map packages"]
    fn the_committed_map_matches_its_packages() {
        crate::same_packages(None).expect("the selected packages match the committed map");
        for (path, made) in crate::generate(None).expect("readable packages") {
            let committed = std::fs::read_to_string(&path).expect("the committed table");
            assert!(
                crate::emit::same(&committed, &made),
                "{path:?} is stale: run tools/import_map"
            );
        }
    }

    #[test]
    fn a_different_package_is_refused_before_decoding() {
        let folder = std::env::temp_dir().join(format!("import_map-{}", std::process::id()));
        let game = folder.join("game/dota");
        let demo = folder.join("game/dota_addons/hero_demo/maps");
        std::fs::create_dir_all(game.join("maps")).expect("scratch map folder");
        std::fs::create_dir_all(&demo).expect("scratch demo folder");
        let lacking = crate::install::game_dir(Some(&folder)).unwrap_err();
        assert!(
            lacking.ends_with("has no game/dota/pak01_dir.vpk"),
            "{lacking}"
        );
        std::fs::write(game.join("pak01_dir.vpk"), b"").expect("package directory");
        std::fs::write(game.join("maps/dota.vpk"), b"new map").expect("wrong map bytes");
        std::fs::write(demo.join("hero_demo_main.vpk"), b"demo").expect("demo bytes");
        let refusal = crate::same_packages(Some(&folder)).unwrap_err();
        assert!(
            refusal.starts_with("map package mismatch: expected "),
            "{refusal}"
        );
        assert!(refusal.contains(", got "), "{refusal}");
        let relative = crate::install::game_dir(Some(std::path::Path::new("dota 2 beta")));
        assert!(relative.unwrap_err().contains("is not an absolute path"));
        std::fs::remove_dir_all(&folder).expect("scratch folder removed");
    }

    #[test]
    fn runs_hold_the_cells_in_order_none_longer_than_a_row() {
        let cells: Vec<u8> = (0..crate::frame::CELLS * 3)
            .map(|i| u8::from(i >= crate::frame::CELLS / 2))
            .collect();
        let runs = crate::ground::runs(&cells, false);
        assert!(
            runs.iter()
                .all(|&(length, _)| usize::from(length) <= crate::frame::CELLS)
        );
        let rebuilt: Vec<u8> = runs
            .iter()
            .flat_map(|&(length, cell)| std::iter::repeat_n(cell, usize::from(length)))
            .collect();
        assert_eq!(rebuilt, cells);
        assert_eq!(runs, [(144, 0), (288, 1), (288, 1), (144, 1)]);
        let by_row = crate::ground::runs(&cells, true);
        assert_eq!(
            by_row,
            [(144, 0), (144, 1), (288, 1), (288, 1)],
            "split at each row's end"
        );
    }
}
