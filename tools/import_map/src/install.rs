//! Opens the packages holding entities, navigation, collision and trigger volumes.

use std::io::Read;
use std::path::{Path, PathBuf};

use heatel::kv3::Value;
use heatel::physics::Physics;
use heatel::world::{EntityLump, GridNav};
use heatel::{Budget, DirSource, Resource, Vpk, find_game_dir};
use sha2::{Digest, Sha256};

/// Most entity lumps one map spawns with: its own and the layers it switches on.
const MAX_LUMPS: usize = 16;
/// Most trigger models one map's volumes name.
const MAX_MODELS: usize = 256;

/// The map source can be selected independently of the other data tools.
pub const MAP_SOURCE_VARIABLE: &str = "BOTA_MAP_SOURCE_PATH";
pub const DOTA_PATH_VARIABLE: &str = "BOTA_DOTA_PATH";

/// Where a map's package lies in an install.
#[derive(Clone, Copy)]
pub struct Package {
    /// The map's name: its package is `<folder>/maps/<name>.vpk`.
    pub name: &'static str,
    /// The folder of `game/` that holds its `maps/`: `dota`, or an addon's `dota_addons/<addon>`.
    pub folder: &'static str,
}

/// Dota's own map.
pub const DOTA: Package = Package {
    name: "dota",
    folder: "dota",
};

/// The hero demo's map, an addon the game ships.
pub const HERO_DEMO: Package = Package {
    name: "hero_demo_main",
    folder: "dota_addons/hero_demo",
};

/// One map package and its decoded data.
pub struct Map {
    /// The checksum of the map package.
    pub fingerprint: String,
    /// The map's name (`dota`): its package is `maps/<name>.vpk`.
    pub name: String,
    /// The entities the map spawns with: those of its own lump, then those of each layer it
    /// switches on at the start, in the order the lump names them.
    pub entities: Vec<Value>,
    /// The navigation grid.
    pub nav: GridNav,
    /// The collision of the map's world: its ground, cliffs and water.
    pub world: Physics,
    source: DirSource,
    package: Vpk,
}

/// The `dota 2 beta` folder `--dota` names, else the one [`DOTA_PATH_VARIABLE`] names.
pub fn install_folder(dota: Option<&Path>) -> Option<PathBuf> {
    dota.map(Path::to_path_buf)
        .or_else(|| std::env::var_os(MAP_SOURCE_VARIABLE).map(PathBuf::from))
        .or_else(|| std::env::var_os(DOTA_PATH_VARIABLE).map(PathBuf::from))
}

/// Resolves the package tree, preferring an explicitly named folder.
pub fn game_dir(dota: Option<&Path>) -> Result<PathBuf, String> {
    let game = match install_folder(dota) {
        None => find_game_dir(None).ok_or(format!(
            "no Dota 2 install found in a Steam library; pass --dota or set {DOTA_PATH_VARIABLE}"
        ))?,
        Some(folder) => {
            if !folder.is_absolute() {
                return Err(format!(
                    "the install {folder:?} is not an absolute path: name the \"dota 2 beta\" \
                     folder in full"
                ));
            }
            let game = folder.join("game/dota");
            if !game.join("pak01_dir.vpk").is_file() {
                return Err(format!(
                    "the folder {folder:?} has no game/dota/pak01_dir.vpk"
                ));
            }
            game
        }
    };
    Ok(game)
}

fn map_directory(game: &Path, package: Package) -> Result<PathBuf, String> {
    Ok(game
        .parent()
        .ok_or("the game folder has no parent")?
        .join(package.folder)
        .join("maps"))
}

pub fn fingerprint(game: &Path, package: Package) -> Result<String, String> {
    let path = map_directory(game, package)?.join(format!("{}.vpk", package.name));
    let mut file = std::fs::File::open(&path).map_err(|error| format!("{path:?}: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("{path:?}: {error}"))?;
    if !metadata.is_file() || metadata.len() > 4 << 30 {
        return Err(format!(
            "{path:?}: expected a map package of at most four GiB"
        ));
    }
    let mut reader = (&mut file).take(metadata.len());
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 << 10];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("{path:?}: {error}"))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    if reader.limit() != 0 {
        return Err(format!("{path:?}: the package size changed while reading"));
    }
    use std::fmt::Write as _;
    let mut checksum = String::with_capacity(64);
    for byte in hash.finalize().iter() {
        write!(checksum, "{byte:02x}").expect("writing to a string");
    }
    Ok(checksum)
}

impl Map {
    /// The map of `package` in the install under `dota` (a `dota 2 beta` folder), else the first
    /// Steam library's.
    pub fn open(dota: Option<&Path>, package: Package) -> Result<Map, String> {
        let name = package.name;
        let game = game_dir(dota)?;
        let fingerprint = fingerprint(&game, package)?;
        let source = DirSource::new(map_directory(&game, package)?);
        let package =
            Vpk::open(&source, &format!("{name}.vpk")).map_err(|e| format!("{name}.vpk: {e}"))?;
        let read = |path: &str| {
            package
                .read_file(&source, path)
                .map_err(|e| format!("{path}: {e}"))
        };
        let nav = GridNav::read(&read(&format!("maps/{name}.gnv"))?).map_err(|e| e.to_string())?;
        let physics = read(&format!("maps/{name}/world_physics.vmdl_c"))?;
        let world = Physics::read(
            &Resource::parse(&physics).map_err(|e| e.to_string())?,
            &mut Budget::default(),
        )
        .map_err(|e| format!("world physics: {e}"))?;
        let mut map = Map {
            fingerprint,
            name: name.to_owned(),
            entities: Vec::new(),
            nav,
            world,
            source,
            package,
        };
        map.entities = map.spawned_entities()?;
        Ok(map)
    }

    /// The entities of the map's lump and of every layer it switches on at the start: the
    /// layers an `info_world_layer` names with its first spawn flag ("start enabled") set.
    fn spawned_entities(&self) -> Result<Vec<Value>, String> {
        let lump = self.lump(&format!("maps/{}/entities/default_ents.vents", self.name))?;
        let mut enabled: Vec<&str> = Vec::new();
        for layer in lump
            .entities
            .iter()
            .filter(|e| e.get("classname").and_then(Value::as_str) == Some("info_world_layer"))
        {
            let name = layer
                .get("layerName")
                .and_then(Value::as_str)
                .ok_or("an info_world_layer names no layerName")?;
            // An absent spawnflags is no flag set: the map compiler leaves the key out of a layer
            // that starts disabled (the destruction layers).
            let flags = match layer.get("spawnflags") {
                None => 0,
                Some(_) => int(layer, "spawnflags")
                    .ok_or(format!("layer {name}: spawnflags not a number"))?,
            };
            if flags & 1 != 0 {
                enabled.push(name);
            }
        }
        for name in &enabled {
            let path = format!("maps/{}/entities/{name}.vents", self.name);
            if !lump.children.contains(&path) {
                return Err(format!(
                    "the enabled layer {name} is not one of the map's lumps"
                ));
            }
        }
        let mut entities = lump.entities.clone();
        let mut lumps = 1;
        for child in &lump.children {
            let layer = child
                .rsplit('/')
                .next()
                .and_then(|file| file.strip_suffix(".vents"))
                .ok_or(format!("{child}: not an entity lump"))?;
            if !enabled.contains(&layer) {
                continue;
            }
            lumps += 1;
            if lumps > MAX_LUMPS {
                return Err("more entity layers than any map has".into());
            }
            entities.extend(self.lump(child)?.entities);
        }
        Ok(entities)
    }

    fn lump(&self, path: &str) -> Result<EntityLump, String> {
        if !path.starts_with(&format!("maps/{}/entities/", self.name)) || path.contains("..") {
            return Err(format!("{path}: outside the map's entities"));
        }
        let bytes = self
            .package
            .read_file(&self.source, &format!("{path}_c"))
            .map_err(|e| format!("{path}: {e}"))?;
        let resource = Resource::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
        EntityLump::read(&resource).map_err(|e| format!("{path}: {e}"))
    }

    /// The collision of a trigger's model (`maps/<map>/entities/<x>.vmdl`), which must be one of
    /// the map's own entity models.
    pub fn volume(&self, model: &str, read: &mut usize) -> Result<Physics, String> {
        *read += 1;
        if *read > MAX_MODELS {
            return Err("more volume models than any map has".into());
        }
        let valid = model.len() <= 256
            && model.starts_with(&format!("maps/{}/entities/", self.name))
            && model.ends_with(".vmdl")
            && model
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_/.-".contains(&c))
            && !model.split('/').any(|part| part == "..");
        if !valid {
            return Err(format!("{model:?}: not a volume model of the map"));
        }
        let bytes = self
            .package
            .read_file(&self.source, &format!("{model}_c"))
            .map_err(|e| format!("{model}: {e}"))?;
        let resource = Resource::parse(&bytes).map_err(|e| format!("{model}: {e}"))?;
        Physics::read(&resource, &mut Budget::default()).map_err(|e| format!("{model}: {e}"))
    }
}

/// An integer key, stored as a number or as its text.
pub fn int(entity: &Value, key: &str) -> Option<i64> {
    match entity.get(key)? {
        Value::String(text) => text.trim().parse().ok(),
        other => other.as_i64(),
    }
}
