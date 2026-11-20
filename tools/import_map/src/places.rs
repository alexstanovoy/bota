//! Every landmark of the map: `crates/bota_core/src/map_data.rs`.

use std::fmt::Write;

use heatel::kv3::Value;
use heatel::physics::Physics;

use crate::entity::{Entity, named, of_class};
use crate::frame::{MAP_SIZE, whole};
use crate::install::{Map, int};
use crate::overrides::{LANE_BENDS, POSITION_OVERRIDES};

/// Most hulls, parts and corners the volumes of one map may have.
const MAX_HULLS: usize = 256;
const MAX_CORNERS: usize = 65_536;

/// How far a secret shop's keeper may stand from its volume's origin.
const SHOP_KEEPER_REACH: f64 = 1000.0;

const LANES: [(&str, &str); 3] = [
    ("mid", "LANE_MID"),
    ("top", "LANE_TOP"),
    ("bot", "LANE_BOT"),
];
const SIDES: [(&str, &str, &str); 2] =
    [("goodguys", "good", "RADIANT"), ("badguys", "bad", "DIRE")];

/// The source of the generated landmark table: the Dota map's, then the hero demo map's.
pub fn table(map: &Map, demo: &Map) -> Result<String, String> {
    let mut out = crate::emit::header(
        "Every landmark of the Dota map and of the hero demo map, as the map packages' entities place \
         them, in bota's frame (Dota's shifted by half the map). Each cites the entity it comes from \
         by its map editor id.",
        &format!("{}/{}", map.fingerprint, demo.fingerprint),
    );
    out.push_str(
        "\nuse bota_proto::{Team, Vec2};\n\nuse crate::rules::{LANE_BOT, LANE_MID, LANE_TOP};\nuse \
         crate::{CampDef, CampKind, OutpostDef, PitZone, RuneSpot, RuneSpotKind, ShopZone, TwinGateDef, \
         BaseBlockerDef};\nuse crate::base_blocker::blocker;\nuse crate::camp::{camp, spawn_box};\nuse \
         crate::objective::twin_gate;\nuse crate::shop::point;\n",
    );
    let e = &map.entities;
    for (id, _) in POSITION_OVERRIDES {
        if !e.iter().any(|entity| Entity(entity).id() == *id) {
            return Err(format!(
                "a position override names {id}, which the map does not have"
            ));
        }
    }
    let mut volumes = 0;
    bases(&mut out, e)?;
    lanes(&mut out, e)?;
    camps(&mut out, map, &mut volumes)?;
    objectives(&mut out, e)?;
    zones(&mut out, map, &mut volumes)?;
    hero_demo(&mut out, &demo.entities)?;
    Ok(out)
}

/// The hero demo map's landmarks: one tower and one wave spawner a side on its one lane, the
/// fountains, and the corners the lane bends through.
fn hero_demo(out: &mut String, e: &[Value]) -> Result<(), String> {
    out.push_str("\n// The hero demo map, `hero_demo_main`.\n");
    for (long, short, side) in SIDES {
        let fountain = named(
            e,
            "ent_dota_fountain",
            &format!("ent_dota_fountain_{short}"),
        )?;
        let _ = write!(
            out,
            "\n/// The hero demo map's {} fountain.\n{}pub const DEMO_{side}_FOUNTAIN_POS: Vec2 = {};\n",
            title(side),
            cite(fountain),
            point_of(fountain)?
        );
        let tower = named(e, "npc_dota_tower", &format!("dota_{long}_tower1_mid"))?;
        let _ = write!(
            out,
            "\n/// The hero demo map's one {} tower.\n{}pub const DEMO_{side}_TOWERS: [(u8, u8, Vec2); 1] = \
             [(LANE_MID, 1, {})];\n",
            title(side),
            cite(tower),
            point_of(tower)?
        );
        let class = format!("npc_dota_spawner_{short}_mid");
        let mut found = of_class(e, &class);
        let spawner = found.next().ok_or(format!("no {class}"))?;
        if found.next().is_some() {
            return Err(format!("two {class}"));
        }
        let _ = write!(
            out,
            "\n/// Where the hero demo map's {} wave appears.\n{}pub const DEMO_{side}_CREEP_SPAWN: Vec2 = {};\n",
            title(side),
            cite(spawner),
            point_of(spawner)?
        );
    }
    let mut corners = Vec::new();
    let mut next = "lane_mid_pathcorner_goodguys_1".to_owned();
    while !next.is_empty() {
        let corner = corner_named(e, &next)?;
        corners.push(corner);
        if corners.len() > 64 {
            return Err("a lane of more than 64 corners".into());
        }
        let target = corner.text("target");
        next = target.strip_prefix("[PR#]").unwrap_or(target).to_owned();
    }
    let [_, inner @ .., _] = &corners[..] else {
        return Err("a lane of fewer than two corners".into());
    };
    let _ = write!(
        out,
        "\n/// The corners the hero demo map's lane bends through between the two towers: the Radiant \
         wave's path corners but its first and last.\npub const DEMO_LANE_CORNERS: [Vec2; {}] = [\n",
        inner.len()
    );
    for corner in inner {
        let _ = writeln!(out, "{}{},", cite(*corner), point_of(*corner)?);
    }
    out.push_str("];\n");
    Ok(())
}

/// `Vec2::from_ints(x, y)` of a point of bota's frame, and the entity it came from.
fn point_of(entity: Entity) -> Result<String, String> {
    let [x, y, _] = entity.at()?;
    let (x, y) = POSITION_OVERRIDES
        .iter()
        .find(|(id, _)| *id == entity.id())
        .map_or((whole(x), whole(y)), |(_, at)| *at);
    Ok(format!("Vec2::from_ints({x}, {y})"))
}

fn cite(entity: Entity) -> String {
    let name = match entity.name() {
        "" => entity.class(),
        name => name,
    };
    format!("// {name} [{}]\n", entity.id())
}

fn bases(out: &mut String, e: &[Value]) -> Result<(), String> {
    for (_, short, side) in SIDES {
        let fountain = named(
            e,
            "ent_dota_fountain",
            &format!("ent_dota_fountain_{short}"),
        )?;
        let _ = write!(
            out,
            "\n/// The {} fountain.\n{}pub const {side}_FOUNTAIN_POS: Vec2 = {};\n",
            title(side),
            cite(fountain),
            point_of(fountain)?
        );
    }
    for (long, _, side) in SIDES {
        let fort = named(e, "npc_dota_fort", &format!("dota_{long}_fort"))?;
        let _ = write!(
            out,
            "\n/// The {} Ancient.\n{}pub const {side}_ANCIENT_POS: Vec2 = {};\n",
            title(side),
            cite(fort),
            point_of(fort)?
        );
    }
    for (long, short, side) in SIDES {
        let _ = write!(
            out,
            "\n/// {} towers: lane, tier and position; mid's tier one to three, top's, bottom's, then \
             the two tier fours.\npub const {side}_TOWERS: [(u8, u8, Vec2); 11] = [\n",
            title(side)
        );
        for (lane, constant) in LANES {
            for tier in 1..=3 {
                let tower = named(
                    e,
                    "npc_dota_tower",
                    &format!("dota_{long}_tower{tier}_{lane}"),
                )?;
                let _ = writeln!(
                    out,
                    "{}({constant}, {tier}, {}),",
                    cite(tower),
                    point_of(tower)?
                );
            }
        }
        for end in ["top", "bot"] {
            let tower = named(e, "npc_dota_tower", &format!("dota_{long}_tower4_{end}"))?;
            let _ = writeln!(out, "{}(LANE_MID, 4, {}),", cite(tower), point_of(tower)?);
        }
        out.push_str("];\n");
        let _ = write!(
            out,
            "\n/// {} barracks: lane, whether it feeds the ranged creeps, and position.\npub const \
             {side}_BARRACKS: [(u8, bool, Vec2); 6] = [\n",
            title(side)
        );
        for (lane, constant) in LANES {
            for (kind, ranged) in [("melee", false), ("range", true)] {
                let rax = named(
                    e,
                    "npc_dota_barracks",
                    &format!("{short}_rax_{kind}_{lane}"),
                )?;
                let _ = writeln!(
                    out,
                    "{}({constant}, {ranged}, {}),",
                    cite(rax),
                    point_of(rax)?
                );
            }
        }
        out.push_str("];\n");
        let mut fillers: Vec<(u32, Entity)> = Vec::new();
        for building in of_class(e, "npc_dota_building") {
            let Some(number) = building.name().strip_prefix(&format!("{short}_filler_")) else {
                continue;
            };
            let number = number
                .parse()
                .map_err(|_| format!("filler {} has no number", building.name()))?;
            fillers.push((number, building));
        }
        fillers.sort_by_key(|(number, _)| *number);
        if fillers.len() != 7 {
            return Err(format!("{} {side} fillers, not 7", fillers.len()));
        }
        let _ = write!(
            out,
            "\n/// {} fillers, by the number in their names.\npub const {side}_FILLERS: [Vec2; 7] = [\n",
            title(side)
        );
        for (_, filler) in fillers {
            let _ = writeln!(out, "{}{},", cite(filler), point_of(filler)?);
        }
        out.push_str("];\n");
    }
    Ok(())
}

fn lanes(out: &mut String, e: &[Value]) -> Result<(), String> {
    for (_, short, side) in SIDES {
        let _ = write!(
            out,
            "\n/// Where {} creep waves appear, by lane: the map's lane spawners.\npub const \
             {side}_CREEP_SPAWNS: [Vec2; 3] = [\n",
            title(side)
        );
        for (lane, _) in LANES {
            let class = format!("npc_dota_spawner_{short}_{lane}");
            let mut found = of_class(e, &class);
            let spawner = found.next().ok_or(format!("no {class}"))?;
            if found.next().is_some() {
                return Err(format!("two {class}"));
            }
            let _ = writeln!(out, "{}{},", cite(spawner), point_of(spawner)?);
        }
        out.push_str("];\n");
    }
    for (name, (x, y)) in LANE_BENDS {
        let lane = if name.starts_with("TOP") {
            "top"
        } else {
            "bottom"
        };
        let _ = write!(
            out,
            "\n/// Where the {lane} lane bends: a hand-chosen corner ([`LANE_BENDS`] of the tool's \
             overrides), not an entity.\npub const {name}: Vec2 = Vec2::from_ints({x}, {y});\n"
        );
    }
    Ok(())
}

fn camps(out: &mut String, map: &Map, volumes: &mut usize) -> Result<(), String> {
    let spawners = by_id(of_class(&map.entities, "npc_dota_neutral_spawner"))?;
    let _ = write!(
        out,
        "\n/// Every camp, in the order of its spawner's map editor id: its `npc_dota_neutral_spawner` (`origin`, `NeutralType`, \
         `AggroType` 1 for pullable, `MinSpawnType` and `MaxSpawnType` the rosters drawn from, 7 for a flooded camp's \
         amphibians, `MaxUpgradeCount` the evolutions) and the box of its spawn volume, the \
         `trigger_multiple` the spawner names: the volume's hull corners about its origin, least and \
         greatest.\npub const CAMPS: [CampDef; {}] = [\n",
        spawners.len()
    );
    for spawner in spawners {
        let kind = match spawner.text("NeutralType") {
            "0" => "Small",
            "1" => "Medium",
            "2" => "Large",
            "3" => "Ancient",
            other => return Err(format!("camp {}: NeutralType {other:?}", spawner.id())),
        };
        let pullable = match spawner.text("AggroType") {
            "0" => false,
            "1" => true,
            other => return Err(format!("camp {}: AggroType {other:?}", spawner.id())),
        };
        let (low, high) = (
            int(spawner.0, "MinSpawnType"),
            int(spawner.0, "MaxSpawnType"),
        );
        let (rosters, evolutions) = match (low, high) {
            (Some(7), Some(7)) => (
                (0, 0),
                int(spawner.0, "MaxUpgradeCount")
                    .ok_or(format!("flooded camp {}: no MaxUpgradeCount", spawner.id()))?,
            ),
            (Some(first @ 1..=6), Some(last @ 1..=6)) if first <= last => ((first, last), 0),
            _ => {
                return Err(format!(
                    "camp {}: spawn types {low:?}..{high:?}",
                    spawner.id()
                ));
            }
        };
        let wanted = spawner.text("VolumeName");
        let volume = named(
            &map.entities,
            "trigger_multiple",
            wanted.strip_prefix("[PR#]").unwrap_or(wanted),
        )?;
        if volume.yaw()? != 0.0 {
            return Err(format!("camp volume {} is turned", volume.id()));
        }
        let [ox, oy, _] = volume.at()?;
        let physics = map.volume(volume.text("model"), volumes)?;
        let corners = corners(&physics)?;
        let least = |axis: usize| {
            corners
                .iter()
                .map(|c| f64::from(c[axis]))
                .fold(f64::INFINITY, f64::min)
        };
        let most = |axis: usize| {
            corners
                .iter()
                .map(|c| f64::from(c[axis]))
                .fold(f64::NEG_INFINITY, f64::max)
        };
        let _ = write!(
            out,
            "// {} [{}], its volume [{}]\ncamp({}, CampKind::{kind}, {pullable}, {rosters:?}, {evolutions}, \
             spawn_box({}, {}, {}, {})),\n",
            volume.name(),
            spawner.id(),
            volume.id(),
            point_of(spawner)?,
            whole(ox + least(0)),
            whole(oy + least(1)),
            whole(ox + most(0)),
            whole(oy + most(1)),
        );
    }
    out.push_str("];\n");
    Ok(())
}

fn objectives(out: &mut String, e: &[Value]) -> Result<(), String> {
    out.push_str("\n/// The Outposts, each held at the start by the side on whose half of the map it stands.\npub const DOTA_OUTPOSTS: [OutpostDef; 2] = [\n");
    let outposts: Vec<Entity> = of_class(e, "npc_dota_watch_tower").collect();
    count(&outposts, 2, "Outposts")?;
    for outpost in outposts {
        let _ = writeln!(
            out,
            "{}OutpostDef {{ at: {}, home: Team::{} }},",
            cite(outpost),
            point_of(outpost)?,
            half(outpost)?
        );
    }
    out.push_str("];\n");
    list(
        out,
        e,
        "npc_dota_lantern",
        "DOTA_WATCHERS",
        "Where the Watchers stand.",
        10,
    )?;
    list(
        out,
        e,
        "npc_dota_xp_fountain",
        "DOTA_SHRINES",
        "Where the Shrines of Wisdom stand.",
        2,
    )?;
    list(
        out,
        e,
        "npc_dota_lotus_pool",
        "DOTA_LOTUS_POOLS",
        "Where the Lotus Pools stand.",
        2,
    )?;
    out.push_str(
        "\n/// The Twin Gates, each carrying to the other; `npc_dota_portal1` takes on the Dire's look, \
         the other the Radiant's.\npub const DOTA_TWIN_GATES: [TwinGateDef; 2] = [\n",
    );
    let gates: Vec<Entity> = of_class(e, "npc_dota_unit_twin_gate").collect();
    count(&gates, 2, "Twin Gates")?;
    for gate in gates {
        let look = match gate.name() {
            "npc_dota_portal1" => "Dire",
            "npc_dota_portal2" => "Radiant",
            other => return Err(format!("Twin Gate {other:?}")),
        };
        let [x, y, _] = gate.at()?;
        let _ = writeln!(
            out,
            "{}twin_gate({}, {}, {}, Team::{look}),",
            cite(gate),
            whole(x * 100.0),
            whole(y * 100.0),
            whole(gate.yaw()?.rem_euclid(360.0) * 100.0)
        );
    }
    out.push_str("];\n");
    out.push_str("\n/// Where runes appear: the river's power-up spots, then each side's jungle Bounty spot.\npub const DOTA_RUNE_SPOTS: [RuneSpot; 4] = [\n");
    let rivers: Vec<Entity> = of_class(e, "dota_item_rune_spawner_powerup").collect();
    let jungles: Vec<Entity> = of_class(e, "dota_item_rune_spawner_bounty").collect();
    count(&rivers, 2, "river rune spots")?;
    count(&jungles, 2, "bounty rune spots")?;
    for river in rivers {
        let _ = writeln!(
            out,
            "{}RuneSpot {{ at: {}, kind: RuneSpotKind::River }},",
            cite(river),
            point_of(river)?
        );
    }
    for jungle in jungles {
        let team = team(jungle.text("RuneTeam"))?;
        let _ = writeln!(
            out,
            "{}RuneSpot {{ at: {}, kind: RuneSpotKind::Jungle(Team::{team}) }},",
            cite(jungle),
            point_of(jungle)?
        );
    }
    out.push_str("];\n");
    for (name, constant, what) in [
        (
            "roshan_location_2",
            "ROSHAN_DAY_PIT",
            "Roshan's pit by day, top-left of the river.",
        ),
        (
            "roshan_location_1",
            "ROSHAN_NIGHT_PIT",
            "Roshan's pit by night, bottom-right of the river.",
        ),
    ] {
        let spot = named(e, "info_player_start_dota", name)?;
        let _ = write!(
            out,
            "\n/// {what}\n{}pub const {constant}: Vec2 = {};\n",
            cite(spot),
            point_of(spot)?
        );
    }
    out.push_str("\n/// The permanent base exits that let their own side through.\npub const DOTA_BASE_BLOCKERS: [BaseBlockerDef; 4] = [\n");
    let blockers: Vec<Entity> = of_class(e, "npc_dota_base_blocker").collect();
    count(&blockers, 4, "base blockers")?;
    for field in blockers {
        let [x, y, z] = field.at()?;
        let brads = whole(field.yaw()?.rem_euclid(360.0) / 360.0 * 65536.0) % 65536;
        let _ = writeln!(
            out,
            "{}blocker({}, {}, {brads}, Team::{}, {}),",
            cite(field),
            whole(x * 100.0),
            whole(y * 100.0),
            team(field.text("teamnumber"))?,
            whole(z * 100.0)
        );
    }
    out.push_str("];\n");
    Ok(())
}

/// The convex volumes of the map's triggers: Roshan's pits, the secret shops, the no-ward zones.
fn zones(out: &mut String, map: &Map, volumes: &mut usize) -> Result<(), String> {
    let e = &map.entities;
    out.push_str("\n/// Roshan's two pits as the map's `trigger_boss_attackable` volumes bound them.\npub const DOTA_BOSS_ZONES: [PitZone; 2] = [\n");
    let pits: Vec<Entity> = of_class(e, "trigger_boss_attackable").collect();
    count(&pits, 2, "boss pits")?;
    for pit in pits {
        for hull in turned_hulls(map, pit, volumes, 1.0)? {
            let _ = writeln!(
                out,
                "{}PitZone {{ corners: &[{}] }},",
                cite(pit),
                ints(&hull)
            );
        }
    }
    out.push_str("];\n");
    out.push_str(
        "\n/// The secret shops, west first: each keeper's spot and its `trigger_shop` volume of shop \
         type 2, in raw fixed-point (16 fraction bits).\npub const DOTA_SECRET_SHOPS: [ShopZone; 2] = [\n",
    );
    let keepers: Vec<Entity> = of_class(e, "ent_dota_shop")
        .filter(|s| s.text("shoptype") == "2")
        .collect();
    let mut shops: Vec<(Entity, Entity)> = Vec::new();
    for trigger in of_class(e, "trigger_shop").filter(|s| s.text("shoptype") == "2") {
        let [tx, ty, _] = trigger.at()?;
        let mut nearest = None;
        for keeper in &keepers {
            let [kx, ky, _] = keeper.at()?;
            let distance = (kx - tx).hypot(ky - ty);
            if nearest.is_none_or(|(d, _)| distance < d) {
                nearest = Some((distance, *keeper));
            }
        }
        let (distance, keeper) = nearest.ok_or("a secret shop volume without a keeper")?;
        // The keeper stands in or by its volume; one further off is another shop's.
        if distance > SHOP_KEEPER_REACH {
            return Err(format!(
                "secret shop volume {}: the nearest keeper stands {distance:.0} units off",
                trigger.id()
            ));
        }
        shops.push((keeper, trigger));
    }
    count(&shops, 2, "secret shops")?;
    let mut keyed = Vec::new();
    for (keeper, trigger) in shops {
        keyed.push((keeper.at()?[0], keeper, trigger));
    }
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, keeper, trigger) in keyed {
        let [x, y, _] = keeper.at()?;
        let hulls = turned_hulls(map, trigger, volumes, 65536.0)?;
        let [hull] = &hulls[..] else {
            return Err(format!(
                "secret shop volume {} is not one hull",
                trigger.id()
            ));
        };
        let corners: Vec<String> = hull
            .iter()
            .map(|(x, y)| format!("point({x}, {y})"))
            .collect();
        let _ = write!(
            out,
            "// the keeper [{}], its volume [{}]\nShopZone {{ pos: point({}, {}), corners: &[{}] }},\n",
            keeper.id(),
            trigger.id(),
            whole(x * 65536.0),
            whole(y * 65536.0),
            corners.join(", ")
        );
    }
    out.push_str("];\n");
    Ok(())
}

/// Entities in the order of their map editor ids (`2699:28170`, numbers compared), which stay
/// with an entity from patch to patch while the lump's own order may not: the order of camps and
/// no-ward volumes is their identity.
fn by_id<'a>(entities: impl Iterator<Item = Entity<'a>>) -> Result<Vec<Entity<'a>>, String> {
    let mut keyed = Vec::new();
    for entity in entities {
        let key = entity
            .id()
            .split(':')
            .map(|part| part.parse::<u64>())
            .collect::<Result<Vec<u64>, _>>()
            .map_err(|_| format!("{} has the editor id {:?}", entity.class(), entity.id()))?;
        keyed.push((key, entity));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    if keyed.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err("two entities share an editor id".into());
    }
    Ok(keyed.into_iter().map(|(_, entity)| entity).collect())
}

/// The path corner of a name. The hero demo map places its Radiant lane's first corner twice, at
/// one spot: copies at one spot are one corner.
fn corner_named<'a>(e: &'a [Value], name: &str) -> Result<Entity<'a>, String> {
    let mut found = of_class(e, "path_corner").filter(|c| c.name() == name);
    let first = found.next().ok_or(format!("no path_corner {name}"))?;
    for other in found {
        if other.at()? != first.at()? {
            return Err(format!("two path_corner {name} at different spots"));
        }
    }
    Ok(first)
}

fn ints(hull: &[(i64, i64)]) -> String {
    hull.iter()
        .map(|(x, y)| format!("Vec2::from_ints({x}, {y})"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every corner of every hull of a volume, in its model's frame.
fn corners(physics: &Physics) -> Result<Vec<[f32; 3]>, String> {
    if !physics.meshes.is_empty() || physics.unread > 0 || physics.hulls.is_empty() {
        return Err("a volume that is not made of hulls".into());
    }
    let out: Vec<[f32; 3]> = physics
        .hulls
        .iter()
        .flat_map(|h| h.vertices.iter().copied())
        .collect();
    if physics.hulls.len() > MAX_HULLS || out.len() > MAX_CORNERS {
        return Err("a volume larger than any map's".into());
    }
    Ok(out)
}

/// Each hull of an entity's volume seen from above: turned by its yaw, moved to its origin,
/// scaled by `unit` and rounded, its outline anticlockwise from its least corner.
fn turned_hulls(
    map: &Map,
    entity: Entity,
    volumes: &mut usize,
    unit: f64,
) -> Result<Vec<Vec<(i64, i64)>>, String> {
    if !entity.unscaled()? {
        return Err(format!("volume {} is scaled", entity.id()));
    }
    let physics = map.volume(entity.text("model"), volumes)?;
    corners(&physics)?;
    let [ox, oy, _] = entity.at()?;
    let (sin, cos) = entity.yaw()?.to_radians().sin_cos();
    let mut out = Vec::new();
    for hull in &physics.hulls {
        let points: Vec<(i64, i64)> = hull
            .vertices
            .iter()
            .map(|v| {
                let (x, y) = (f64::from(v[0]), f64::from(v[1]));
                (
                    whole((ox + x * cos - y * sin) * unit),
                    whole((oy + x * sin + y * cos) * unit),
                )
            })
            .collect();
        let outline = outline(points);
        if outline.len() < 3 {
            return Err(format!("volume {} has a flat hull", entity.id()));
        }
        out.push(outline);
    }
    Ok(out)
}

/// The convex outline of points, anticlockwise from the least.
fn outline(mut points: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    let cross = |a: (i64, i64), b: (i64, i64), c: (i64, i64)| {
        i128::from(b.0 - a.0) * i128::from(c.1 - a.1)
            - i128::from(b.1 - a.1) * i128::from(c.0 - a.0)
    };
    points.sort_unstable();
    points.dedup();
    let mut hull = Vec::new();
    for half in [points.clone(), points.into_iter().rev().collect()] {
        let start = hull.len();
        for point in half {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0
            {
                hull.pop();
            }
            hull.push(point);
        }
        hull.pop();
    }
    hull
}

fn list(
    out: &mut String,
    e: &[Value],
    class: &str,
    constant: &str,
    what: &str,
    expected: usize,
) -> Result<(), String> {
    let found: Vec<Entity> = of_class(e, class).collect();
    count(&found, expected, class)?;
    let _ = write!(
        out,
        "\n/// {what}\npub const {constant}: [Vec2; {expected}] = [\n"
    );
    for entity in found {
        let _ = writeln!(out, "{}{},", cite(entity), point_of(entity)?);
    }
    out.push_str("];\n");
    Ok(())
}

fn count<T>(found: &[T], expected: usize, what: &str) -> Result<(), String> {
    match found.len() == expected {
        true => Ok(()),
        false => Err(format!("{} {what}, not {expected}", found.len())),
    }
}

/// The side whose half of the map a point lies in: the Radiant's below the diagonal from the
/// north-west corner to the south-east one.
fn half(entity: Entity) -> Result<&'static str, String> {
    let [x, y, _] = entity.at()?;
    match x + y {
        sum if sum < f64::from(MAP_SIZE) => Ok("Radiant"),
        sum if sum > f64::from(MAP_SIZE) => Ok("Dire"),
        _ => Err(format!(
            "{} {} stands on the diagonal",
            entity.class(),
            entity.id()
        )),
    }
}

fn team(number: &str) -> Result<&'static str, String> {
    match number {
        "2" => Ok("Radiant"),
        "3" => Ok("Dire"),
        other => Err(format!("team {other:?}")),
    }
}

fn title(side: &str) -> &'static str {
    match side {
        "RADIANT" => "Radiant",
        _ => "Dire",
    }
}
