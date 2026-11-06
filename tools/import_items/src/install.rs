//! What the tool reads of an install: its build, its item scripts and its hero scripts.

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

/// Reads the item scripts of the install under `dota` (a `dota 2 beta` folder), else the first
/// Steam library's.
pub fn read(dota: Option<&Path>) -> Result<Source, String> {
    let package = Package::open(dota)?;
    let items = package.text("scripts/npc/items.txt")?;
    Source::new(package.build, &items)
}

/// The hero scripts of one client build: the base hero every hero falls back on, each hero
/// asked for, and the shared ability list the generic talents stand in.
pub struct HeroSource {
    /// As [`Source::build`].
    pub build: String,
    /// `npc_dota_hero_base.txt`, whose root holds `DOTAHeroes` with the base hero.
    base: Kv,
    /// Each hero's key and its file, whose root holds `DOTAHeroes` with the hero, in the order
    /// asked for.
    heroes: Vec<(String, Kv)>,
    /// `npc_abilities.txt`, whose root holds a `DOTAAbilities` block.
    abilities: Kv,
}

impl HeroSource {
    /// The heroes of the texts of `npc_dota_hero_base.txt`, each hero's own file (by its key),
    /// and `npc_abilities.txt`, read from `build`.
    pub fn new(
        build: String,
        base_txt: &str,
        heroes: &[(&str, &str)],
        abilities_txt: &str,
    ) -> Result<HeroSource, String> {
        let parse = |name: &str, txt: &str| parse_kv(txt).map_err(|e| format!("{name}: {e}"));
        let source = HeroSource {
            build,
            base: parse("npc_dota_hero_base.txt", base_txt)?,
            heroes: heroes
                .iter()
                .map(|(key, txt)| Ok(((*key).to_owned(), parse(key, txt)?)))
                .collect::<Result<_, String>>()?,
            abilities: parse("npc_abilities.txt", abilities_txt)?,
        };
        source
            .base()
            .block()
            .ok_or(format!("npc_dota_hero_base.txt: no {BASE_HERO} block"))?;
        for (key, _) in source.heroes() {
            source
                .hero(key)
                .and_then(Kv::block)
                .ok_or(format!("{key}.txt: no DOTAHeroes {key} block"))?;
        }
        source
            .abilities
            .get("DOTAAbilities")
            .and_then(Kv::block)
            .ok_or("npc_abilities.txt: no DOTAAbilities block")?;
        Ok(source)
    }

    /// `npc_dota_hero_base`'s block.
    pub fn base(&self) -> &Kv {
        static NONE: Kv = Kv::Text(String::new());
        heroes_entry(&self.base, BASE_HERO).unwrap_or(&NONE)
    }

    /// The keys of the heroes asked for, in that order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.heroes.iter().map(|(key, _)| key.as_str())
    }

    /// The block of the hero `key`, one of those asked for.
    pub fn hero(&self, key: &str) -> Option<&Kv> {
        let (_, root) = self.heroes.iter().find(|(asked, _)| asked == key)?;
        heroes_entry(root, key)
    }

    /// The heroes asked for, each by its key and its block.
    fn heroes(&self) -> impl Iterator<Item = (&str, Option<&Kv>)> {
        self.keys().map(|key| (key, self.hero(key)))
    }

    /// The block of an ability of `npc_abilities.txt`, a generic talent's among them.
    pub fn shared_ability(&self, key: &str) -> Option<&Kv> {
        self.abilities.get("DOTAAbilities")?.get(key)
    }
}

/// The hero every hero's script falls back on.
pub const BASE_HERO: &str = "npc_dota_hero_base";

/// The block of `key` in a hero file's `DOTAHeroes` block.
fn heroes_entry<'a>(root: &'a Kv, key: &str) -> Option<&'a Kv> {
    root.get("DOTAHeroes")?.get(key)
}

/// Reads every hero of `keys` and the shared abilities of the install under `dota`.
pub fn read_heroes(dota: Option<&Path>, keys: &[&str]) -> Result<HeroSource, String> {
    let package = Package::open(dota)?;
    let base = package.text("scripts/npc/heroes/npc_dota_hero_base.txt")?;
    let mut heroes = Vec::with_capacity(keys.len());
    for key in keys {
        heroes.push((
            *key,
            package.text(&format!("scripts/npc/heroes/{key}.txt"))?,
        ));
    }
    let abilities = package.text("scripts/npc/npc_abilities.txt")?;
    let heroes: Vec<(&str, &str)> = heroes.iter().map(|(k, t)| (*k, t.as_str())).collect();
    HeroSource::new(package.build, &base, &heroes, &abilities)
}

/// An install's main package.
struct Package {
    build: String,
    source: DirSource,
    vpk: Vpk,
}

impl Package {
    /// The package of the install under `dota`, else the first Steam library's.
    fn open(dota: Option<&Path>) -> Result<Package, String> {
        let game = find_game_dir(dota).ok_or("no Dota 2 install found; pass --dota")?;
        let build = build(&game.join("steam.inf"))?;
        let source = DirSource::new(&game);
        let vpk = Vpk::open(&source, "pak01_dir.vpk").map_err(|e| format!("pak01_dir.vpk: {e}"))?;
        Ok(Package { build, source, vpk })
    }

    /// The text of the file at `path` in the package.
    fn text(&self, path: &str) -> Result<String, String> {
        let bytes = self
            .vpk
            .read_file(&self.source, path)
            .map_err(|e| format!("{path}: {e}"))?;
        Ok(text(&bytes).map_err(|e| format!("{path}: {e}"))?.to_owned())
    }
}

/// The script of one hero's abilities in the install under `dota` (else the first Steam
/// library's): `scripts/npc/heroes/npc_dota_hero_<hero>.txt`, parsed.
#[cfg(test)]
pub fn hero_script(dota: Option<&Path>, hero: &str) -> Result<Kv, String> {
    let game = find_game_dir(dota).ok_or("no Dota 2 install found; pass --dota")?;
    let source = DirSource::new(&game);
    let vpk = Vpk::open(&source, "pak01_dir.vpk").map_err(|e| format!("pak01_dir.vpk: {e}"))?;
    let path = format!("scripts/npc/heroes/npc_dota_hero_{hero}.txt");
    let bytes = vpk
        .read_file(&source, &path)
        .map_err(|e| format!("{path}: {e}"))?;
    parse_kv(text(&bytes).map_err(|e| format!("{path}: {e}"))?).map_err(|e| format!("{path}: {e}"))
}

/// `ClientVersion <n> (<VersionDate>)` of a `steam.inf`.
pub(crate) fn build(path: &Path) -> Result<String, String> {
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
