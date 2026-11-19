//! `import_items`: writes `crates/bota_core/src/data/items.rs` from the `scripts/npc/items.txt`
//! of a local Dota 2 install, for every item the id registry names, and
//! `crates/bota_core/src/heroes_data.rs` from the hero scripts, for every hero of [`HEROES`]. A
//! development tool, never a build step: the tables it writes are committed, hold numbers and
//! Dota's internal names only, and name the client build they were read from.
//!
//! ```text
//! import_items [--dota <"dota 2 beta" folder>] [--check] [--add <item key>]... [--creeps-only]
//! ```
//!
//! `--add` appends a registry line for each new item key first. `--check` writes nothing and
//! fails when a committed table holds other rows than the install gives (the client build it
//! names aside). `--creeps-only` updates or checks the lane creep numeric table alone.
//!
//! Without `--dota`, `BOTA_DOTA_PATH` names the install (a `dota 2 beta` folder), else the first
//! Steam library's is read; the tests read the same one.

mod ability;
mod boss;
mod creeps;
mod dec;
mod emit;
mod emit_heroes;
mod hero;
mod install;
mod item;
mod name;
mod overrides;
mod registry;
mod units;
mod wave_rules;
mod zones;

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

/// The committed hero table.
fn heroes_path() -> PathBuf {
    repo().join("crates/bota_core/src/heroes_data.rs")
}

/// The client build the committed tables were read from, as their `Read from` line names it:
/// the install the checks hold them to. A run on another build writes that build's name;
/// update this with the tables.
const TABLES_BUILD: &str = "client 6983 (Nov 18 2026)";

/// The heroes whose numbers bota reads from the install. A hero not named here keeps its
/// numbers in its own module until it moves over.
const HEROES: &[&str] = &["npc_dota_hero_pudge", "npc_dota_hero_nevermore"];

/// The id registry.
fn registry_path() -> PathBuf {
    repo().join("crates/bota_proto/registry/ids.txt")
}

/// The install `BOTA_DOTA_PATH` names, if it names one: what the tool and its tests read when no
/// `--dota` is given, so a check can be held to a chosen client build rather than whichever one
/// Steam installed last.
fn pinned_install() -> Option<PathBuf> {
    std::env::var_os("BOTA_DOTA_PATH").map(PathBuf::from)
}

struct Args {
    dota: Option<PathBuf>,
    check: bool,
    add: Vec<String>,
    creeps_only: bool,
}

fn args() -> Result<Args, String> {
    let mut parsed = Args {
        dota: pinned_install(),
        check: false,
        add: Vec::new(),
        creeps_only: false,
    };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--dota" => parsed.dota = Some(words.next().ok_or("--dota needs a folder")?.into()),
            "--check" => parsed.check = true,
            "--creeps-only" => parsed.creeps_only = true,
            "--add" => parsed
                .add
                .push(words.next().ok_or("--add needs an item key")?),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(parsed)
}

/// Why a committed table differs from what the install gives: both client builds named, the
/// one the table was read from and the install's.
fn stale(path: &Path, committed: &str, build: &str) -> String {
    let table_build = committed
        .lines()
        .find_map(|line| line.strip_prefix(emit::BUILD_LINE))
        .map_or("no build named".to_owned(), |b| {
            b.trim_end_matches('.').to_owned()
        });
    format!(
        "{path:?} differs from what the install gives: the table was read from {table_build}, \
         the install is {build}; run import_items"
    )
}

fn run() -> Result<(), String> {
    let args = args()?;
    if args.creeps_only && !args.add.is_empty() {
        return Err("--creeps-only and --add exclude each other".into());
    }
    let game = install::game_dir(args.dota.as_deref())?;
    let build = install::build(&game.join("steam.inf"))?;
    println!("import_items: reading {} ({build})", game.display());
    if build != TABLES_BUILD {
        if args.check {
            return Err(format!(
                "the committed tables were read from {TABLES_BUILD}; the install at {} is {build}",
                game.display()
            ));
        }
        println!(
            "import_items: the tables were read from {TABLES_BUILD}; set TABLES_BUILD to {build:?}"
        );
    }
    let creeps = rustfmt(&creeps::table(args.dota.as_deref())?)?;
    let path = repo().join("crates/bota_core/src/creep_data.rs");
    if args.check {
        let committed = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if !emit::same_items(&committed, &creeps) {
            return Err(stale(&path, &committed, &build));
        }
    } else {
        std::fs::write(&path, creeps).map_err(|e| e.to_string())?;
    }
    if args.creeps_only {
        return Ok(());
    }
    let boss = rustfmt(&boss::table(args.dota.as_deref())?)?;
    let boss_path = repo().join("crates/bota_core/src/roshan_data.rs");
    if args.check {
        let committed =
            std::fs::read_to_string(&boss_path).map_err(|e| format!("{boss_path:?}: {e}"))?;
        if !emit::same_items(&committed, &boss) {
            return Err(stale(&boss_path, &committed, &build));
        }
    } else {
        std::fs::write(&boss_path, boss).map_err(|e| format!("{boss_path:?}: {e}"))?;
    }
    let units = rustfmt(&units::table(args.dota.as_deref())?)?;
    let units_path = repo().join("crates/bota_core/src/unit_data.rs");
    if args.check {
        let committed =
            std::fs::read_to_string(&units_path).map_err(|e| format!("{units_path:?}: {e}"))?;
        if !emit::same_items(&committed, &units) {
            return Err(stale(&units_path, &committed, &build));
        }
    } else {
        std::fs::write(&units_path, units).map_err(|e| format!("{units_path:?}: {e}"))?;
    }
    let zones = rustfmt(&zones::table(args.dota.as_deref())?)?;
    let zones_path = repo().join("crates/bota_core/src/placement_data.rs");
    if args.check {
        let committed = std::fs::read_to_string(&zones_path).map_err(|e| e.to_string())?;
        if committed != zones {
            return Err(stale(&zones_path, &committed, &build));
        }
    } else {
        std::fs::write(&zones_path, zones).map_err(|e| e.to_string())?;
    }
    let waves = rustfmt(&wave_rules::table(args.dota.as_deref())?)?;
    let wave_path = repo().join("crates/bota_core/src/wave_data.rs");
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
    let heroes = rustfmt(&emit_heroes::table(&install::read_heroes(
        args.dota.as_deref(),
        HEROES,
    )?)?)?;
    for (path, made) in [
        (table_path(), table),
        (heroes_path(), heroes),
        (wave_path, waves),
    ] {
        if args.check {
            let committed = std::fs::read_to_string(&path).map_err(|e| format!("{path:?}: {e}"))?;
            if !emit::same_items(&committed, &made) {
                return Err(stale(&path, &committed, &build));
            }
            continue;
        }
        std::fs::write(&path, made).map_err(|e| format!("{path:?}: {e}"))?;
    }
    Ok(())
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
    /// Fails, naming both builds and the install, unless the install the checks read is the
    /// build the committed tables were read from ([`TABLES_BUILD`](crate::TABLES_BUILD)).
    fn assert_the_install_is_the_tables_build() {
        let game =
            crate::install::game_dir(crate::pinned_install().as_deref()).expect("an install");
        let build = crate::install::build(&game.join("steam.inf")).expect("its build");
        assert_eq!(
            build,
            crate::TABLES_BUILD,
            "the committed tables were read from {}; the install at {} is {build}",
            crate::TABLES_BUILD,
            game.display()
        );
    }

    /// Every committed table names the build [`TABLES_BUILD`](crate::TABLES_BUILD) says.
    #[test]
    fn every_committed_table_names_the_build_it_was_read_from() {
        for file in [
            "crates/bota_core/src/data/items.rs",
            "crates/bota_core/src/heroes_data.rs",
            "crates/bota_core/src/creep_data.rs",
            "crates/bota_core/src/roshan_data.rs",
            "crates/bota_core/src/unit_data.rs",
        ] {
            let text = std::fs::read_to_string(crate::repo().join(file)).expect("a table");
            let named = text
                .lines()
                .find_map(|line| line.strip_prefix(crate::emit::BUILD_LINE))
                .expect("its build line");
            assert_eq!(named, format!("{}.", crate::TABLES_BUILD), "{file}");
        }
    }

    /// The values of bota's heroes' abilities an AoE bonus widens, as `bota_core`'s
    /// `AOE_ABILITY_VALUES` lists them: each `("ability", "value"),` line of `aoe.rs`'s table.
    fn bota_aoe_values() -> std::collections::BTreeSet<(String, String)> {
        let source = std::fs::read_to_string(crate::repo().join("crates/bota_core/src/aoe.rs"))
            .expect("bota_core's aoe.rs");
        let table = source
            .split("AOE_ABILITY_VALUES")
            .nth(1)
            .and_then(|rest| rest.split("];").next())
            .expect("the table");
        table
            .lines()
            .filter_map(|line| {
                let mut quoted = line.split('"').skip(1).step_by(2);
                Some((quoted.next()?.to_owned(), quoted.next()?.to_owned()))
            })
            .collect()
    }

    /// The hero values bota widens by an AoE bonus are those the install's scripts of its heroes
    /// mark `affected_by_aoe_increase`, no more and no fewer: a new client build that marks or
    /// unmarks one fails here until bota reads it through `widened` (or stops).
    #[test]
    #[ignore = "needs a Dota 2 install"]
    fn the_hero_values_an_aoe_bonus_widens_are_the_installs() {
        assert_the_install_is_the_tables_build();
        use heatel::keyvalues::Kv;
        let mut marked = std::collections::BTreeSet::new();
        for hero in ["nevermore", "pudge"] {
            let root = crate::install::hero_script(crate::pinned_install().as_deref(), hero)
                .expect("the hero's script");
            let abilities = root
                .get("DOTAHeroes")
                .and_then(|heroes| heroes.get(&format!("npc_dota_hero_{hero}")))
                .and_then(|block| block.get("AbilityDefinitions"))
                .and_then(Kv::block)
                .expect("its abilities");
            for (ability, def) in abilities {
                let Some(values) = def.get("AbilityValues").and_then(Kv::block) else {
                    continue;
                };
                for (key, value) in values {
                    if matches!(value.get("affected_by_aoe_increase"), Some(Kv::Text(flag)) if flag.trim() == "1")
                    {
                        marked.insert((ability.clone(), key.clone()));
                    }
                }
            }
        }
        assert_eq!(marked, bota_aoe_values());
    }

    /// The committed table is what the install on this machine gives, the build it names aside:
    /// a new client build that changes an item fails here until the table is regenerated, or
    /// the 7.41f number is kept by an override.
    #[test]
    #[ignore = "needs a Dota 2 install"]
    fn the_committed_table_is_what_the_install_gives() {
        assert_the_install_is_the_tables_build();
        let creeps = crate::rustfmt(
            &crate::creeps::table(crate::pinned_install().as_deref()).expect("lane creep scripts"),
        )
        .expect("formatted creep data");
        let committed =
            std::fs::read_to_string(crate::repo().join("crates/bota_core/src/creep_data.rs"))
                .expect("committed creep data");
        assert!(
            crate::emit::same_items(&committed, &creeps),
            "lane creep script data is stale"
        );
        let boss = crate::rustfmt(
            &crate::boss::table(crate::pinned_install().as_deref()).expect("boss scripts"),
        )
        .expect("formatted scripts");
        let committed =
            std::fs::read_to_string(crate::repo().join("crates/bota_core/src/roshan_data.rs"))
                .expect("committed boss data");
        assert!(
            crate::emit::same_items(&committed, &boss),
            "Roshan data is stale"
        );
        let units = crate::rustfmt(
            &crate::units::table(crate::pinned_install().as_deref()).expect("unit scripts"),
        )
        .expect("formatted unit data");
        let committed =
            std::fs::read_to_string(crate::repo().join("crates/bota_core/src/unit_data.rs"))
                .expect("committed unit data");
        assert!(
            crate::emit::same_items(&committed, &units),
            "courier and ward data is stale"
        );
        let zones = crate::rustfmt(
            &crate::zones::table(crate::pinned_install().as_deref())
                .expect("map placement volumes"),
        )
        .expect("formatted volumes");
        let committed =
            std::fs::read_to_string(crate::repo().join("crates/bota_core/src/placement_data.rs"))
                .expect("committed placement data");
        assert_eq!(committed, zones, "map placement data is stale");
        let source = crate::install::read(crate::pinned_install().as_deref()).expect("an install");
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

    /// The committed hero table is what the install on this machine gives, the build it names
    /// aside: a new client build that changes a hero fails here until the table is regenerated.
    #[test]
    #[ignore = "needs a Dota 2 install"]
    fn the_committed_hero_table_is_what_the_install_gives() {
        assert_the_install_is_the_tables_build();
        let source = crate::install::read_heroes(crate::pinned_install().as_deref(), crate::HEROES)
            .expect("an install");
        let table = crate::emit_heroes::table(&source).expect("importable heroes");
        let table = crate::rustfmt(&table).expect("rustfmt");
        let committed = std::fs::read_to_string(crate::heroes_path()).expect("the committed table");
        assert!(
            crate::emit::same_items(&committed, &table),
            "the committed hero table is stale: run import_items"
        );
    }

    /// The install's secret-shop components, all sixteen of which this catalogue registers. Pin
    /// their intersection with our live ids, then compare both the importer and committed rows.
    #[test]
    #[ignore = "needs a Dota 2 install"]
    fn registered_secret_items_match_the_installs_secret_shop_keys() {
        assert_the_install_is_the_tables_build();
        use heatel::keyvalues::Kv;
        use std::collections::BTreeSet;
        let source = crate::install::read(crate::pinned_install().as_deref()).expect("an install");
        let registry = std::fs::read_to_string(crate::registry_path()).expect("registry");
        let entries = crate::registry::items(&registry).expect("registered ids");
        let expected = BTreeSet::from([
            "item_demon_edge",
            "item_eagle",
            "item_energy_booster",
            "item_hyperstone",
            "item_mystic_staff",
            "item_platemail",
            "item_point_booster",
            "item_reaver",
            "item_relic",
            "item_ring_of_health",
            "item_ring_of_tarrasque",
            "item_talisman_of_evasion",
            "item_tiara_of_selemene",
            "item_ultimate_orb",
            "item_vitality_booster",
            "item_void_stone",
        ]);
        let installed: BTreeSet<_> = entries
            .iter()
            .filter(|entry| !entry.retired)
            .filter(|entry| {
                matches!(source.items().get(&entry.key).and_then(|item| item.get("SecretShop")),
                Some(Kv::Text(value)) if value == "1")
            })
            .map(|entry| entry.key.as_str())
            .collect();
        assert_eq!(
            installed, expected,
            "7.41f secret set among our live catalogue ids"
        );
        let emitted = crate::emit::table(&source, &entries).expect("importable data");
        let committed = std::fs::read_to_string(crate::table_path()).expect("committed data");
        for table in [&emitted, &committed] {
            let flagged: BTreeSet<_> = table
                .split("ItemData = ItemData {")
                .skip(1)
                .filter(|row| row.lines().any(|line| line.trim() == "secret_shop: true,"))
                .map(|row| {
                    row.lines()
                        .find_map(|line| line.trim().strip_prefix("key: \""))
                        .and_then(|key| key.strip_suffix("\","))
                        .expect("row key")
                })
                .collect();
            assert_eq!(
                flagged, installed,
                "secret flags agree with the install's keys"
            );
        }
    }
}
