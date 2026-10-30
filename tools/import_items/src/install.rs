//! What the tool reads of an install: its build, and its item scripts.

use std::path::Path;

use heatel::keyvalues::{Kv, parse_kv, text};
use heatel::vpk::Vpk;
use heatel::{DirSource, find_game_dir};

/// The item scripts of one client build.
pub struct Source {
    /// The client build, as `steam.inf` names it: `ClientVersion` and `VersionDate`.
    pub build: String,
    /// `scripts/npc/items.txt`, whose root holds a `DOTAAbilities` block.
    root: Kv,
}

impl Source {
    /// The scripts of `items_txt` (the text of an `items.txt`), read from `build`.
    pub fn new(build: String, items_txt: &str) -> Result<Source, String> {
        let root = parse_kv(items_txt).map_err(|e| format!("items.txt: {e}"))?;
        let source = Source { build, root };
        source
            .items()
            .block()
            .ok_or("items.txt: no DOTAAbilities block")?;
        Ok(source)
    }

    /// The `DOTAAbilities` block: one block an item, by its key.
    pub fn items(&self) -> &Kv {
        static NONE: Kv = Kv::Text(String::new());
        self.root.get("DOTAAbilities").unwrap_or(&NONE)
    }
}

/// Longest `steam.inf` read; Dota's is a few hundred bytes.
const MAX_STEAM_INF: u64 = 64 << 10;

/// Reads the install under `dota` (a `dota 2 beta` folder), else the first Steam library's.
pub fn read(dota: Option<&Path>) -> Result<Source, String> {
    let game = find_game_dir(dota).ok_or("no Dota 2 install found; pass --dota")?;
    let build = build(&game.join("steam.inf"))?;
    let source = DirSource::new(&game);
    let vpk = Vpk::open(&source, "pak01_dir.vpk").map_err(|e| format!("pak01_dir.vpk: {e}"))?;
    let bytes = vpk
        .read_file(&source, "scripts/npc/items.txt")
        .map_err(|e| format!("items.txt: {e}"))?;
    Source::new(build, text(&bytes).map_err(|e| format!("items.txt: {e}"))?)
}

/// `ClientVersion <n> (<VersionDate>)` of a `steam.inf`.
fn build(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{path:?}: {e}"))?
        .len();
    if size > MAX_STEAM_INF {
        return Err(format!("{path:?}: {size} bytes"));
    }
    let inf = std::fs::read_to_string(path).map_err(|e| format!("{path:?}: {e}"))?;
    let field = |name: &str| {
        inf.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .map(str::trim)
            .filter(|value| {
                !value.is_empty() && value.chars().all(|c| c.is_ascii_graphic() || c == ' ')
            })
            .ok_or(format!("{path:?}: no {name}"))
    };
    Ok(format!(
        "client {} ({})",
        field("ClientVersion")?,
        field("VersionDate")?
    ))
}
