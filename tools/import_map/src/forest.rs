//! The forest and the vision blocker walls: `crates/bota_server/src/game/config/dota_map.rs`, with
//! the ground's cells.

use std::fmt::Write;

use crate::entity::{Entity, of_class};
use crate::frame::{on_map, whole};
use crate::install::Map;

/// What a map's generated tables are called.
pub struct Names {
    /// The map, as the tables' docs name it.
    pub map: &'static str,
    /// Whether its trees are ordered by their text rather than by their coordinates.
    pub trees_by_text: bool,
    /// Whether its terrain's runs stop at each row's end.
    pub runs_by_row: bool,
    pub terrain: &'static str,
    pub forest: &'static str,
    pub walls: &'static str,
}

/// The Dota map's tables.
pub const DOTA_NAMES: Names = Names {
    map: "Dota map",
    trees_by_text: true,
    runs_by_row: false,
    terrain: "TERRAIN_RLE",
    forest: "DOTA_TREES",
    walls: "FOW_BLOCKERS",
};

/// The hero demo map's tables.
pub const DEMO_NAMES: Names = Names {
    map: "hero demo map",
    trees_by_text: false,
    runs_by_row: true,
    terrain: "DEMO_TERRAIN_RLE",
    forest: "DEMO_TREES",
    walls: "DEMO_FOW_BLOCKERS",
};

/// A vision blocker wall: its name and its nodes.
pub type Wall = (String, Vec<(i16, i16)>);

/// Most trees a map may have.
const MAX_TREES: usize = 8192;

/// Every tree the map spawns with, in bota's frame, in the order tree identities have had since
/// the map's table was first made: by their coordinates, or (the Dota map) by their text.
pub fn trees(map: &Map, by_text: bool) -> Result<Vec<(i16, i16)>, String> {
    let mut trees: Vec<(i16, i16)> = Vec::new();
    for tree in of_class(&map.entities, "ent_dota_tree") {
        let [x, y, _] = tree.at()?;
        let (x, y) = (whole(x), whole(y));
        if !on_map(x, y) {
            return Err(format!("tree {} at ({x}, {y}) is off the map", tree.id()));
        }
        trees.push((x as i16, y as i16));
        if trees.len() > MAX_TREES {
            return Err("more trees than any map has".into());
        }
    }
    match by_text {
        true => trees.sort_by_key(|tree| format!("{tree:?}")),
        false => trees.sort_unstable(),
    }
    Ok(trees)
}

/// The map's vision blocker walls: the nodes of each named group of `ent_fow_blocker_node`, in
/// the map's order, the groups in the order their first nodes come. Every node is kept, those
/// beyond the map's edge too: a wall from one of them into the map closes the cells it crosses.
pub fn blockers(map: &Map) -> Result<Vec<Wall>, String> {
    let mut groups: Vec<Wall> = Vec::new();
    for node in of_class(&map.entities, "ent_fow_blocker_node") {
        let name = Entity::name(&node);
        if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
            return Err(format!("blocker node {} has the name {name:?}", node.id()));
        }
        let [x, y, _] = node.at()?;
        let (x, y) = (whole(x), whole(y));
        let at = match groups.iter().position(|(group, _)| group == name) {
            Some(at) => at,
            None => {
                groups.push((name.to_owned(), Vec::new()));
                groups.len() - 1
            }
        };
        let fits =
            |v: i64| i16::try_from(v).map_err(|_| format!("blocker node {} at {v}", node.id()));
        groups[at].1.push((fits(x)?, fits(y)?));
    }
    Ok(groups)
}

/// The source of the generated server table.
pub fn table(
    names: &Names,
    source: &str,
    runs: &[(u16, u8)],
    trees: &[(i16, i16)],
    blockers: &[Wall],
) -> String {
    let Names {
        map,
        terrain,
        forest,
        walls,
        ..
    } = *names;
    let mut out = crate::emit::header(
        &format!("The {map}'s ground, forest and vision blockers, as the map package has them."),
        source,
    );
    out.push_str(&format!(
        "\n/// Run-length encoded terrain cells of the {map}, row-major from the map's south-west \
         corner, one byte per 64-unit cell: bit 7 is ground the navigation grid calls walkable, bit \
         6 is river water, the low five bits are the height step in the map's own steps (128 \
         units) from its lowest ground: the highest of the world's solid (not water) collision over the \
         cell's centre, a half step rounded up. \
         No run is longer than a row{}.\npub const {terrain}: &[(u16, u8)] = &[\n",
        if names.runs_by_row { " or crosses a row's end" } else { "" }
    ));
    for (length, cell) in runs {
        let _ = writeln!(out, "({length}, {cell:#04x}),");
    }
    let _ = write!(
        out,
        "];\n\n/// Every tree of the {map} in this map's frame, the trees of the layers it switches \
         on at the start included.\npub const {forest}: &[(i16, i16)] = &[\n",
    );
    for (x, y) in trees {
        let _ = writeln!(out, "({x}, {y}),");
    }
    let _ = write!(
        out,
        "];\n\n/// The {map}'s own vision blocker polylines, in this map's frame. A sight line \
         crossing a segment between two nodes no further apart than \
         [`FOW_BLOCKER_SPAN`](bota_core::rules::FOW_BLOCKER_SPAN) is blocked, whichever side \
         looks.\npub const {walls}: &[&[(i16, i16)]] = &[\n",
    );
    for (name, nodes) in blockers {
        let _ = writeln!(out, "// {name}\n&[");
        for (x, y) in nodes {
            let _ = writeln!(out, "({x}, {y}),");
        }
        out.push_str("],\n");
    }
    out.push_str("];\n");
    out
}
