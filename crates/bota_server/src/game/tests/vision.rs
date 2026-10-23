//! Who sees what, and what each side is told.

use bota_core::{CAMPS, CellGrid, MAPS, MELEE_CREEP, RANGED_CREEP, map_of, rules};

use crate::game::{Entity, EventVisibility, Health, World, wire_id};
use bota_proto::{Fixed, HeroId, ItemId, MapId, Target, Team, UnitKind, Vec2};

use super::support::*;

#[test]
fn a_world_built_on_a_map_stands_its_buildings_full() {
    let map = map_of(MapId(1));
    let world = World::on_map(map);
    assert_eq!(
        world.entities.len(),
        map_buildings(map),
        "every building and nothing else"
    );
    let view = world.view_full();
    assert_eq!(view.units.len(), world.entities.len());
    let tower = view
        .units
        .iter()
        .find(|u| u.kind == UnitKind::Tower)
        .expect("a tower stands");
    assert_eq!(tower.hp, rules::TOWER_TIER_HP[0], "built and full");
    assert_eq!(tower.max_hp, rules::TOWER_TIER_HP[0]);
    assert!(
        !view.units.iter().any(|u| u.kind == UnitKind::Ancient),
        "no Ancient stands on the demo map"
    );
}

#[test]
fn both_sides_are_always_told_of_every_building() {
    let map = map_of(MapId(1));
    let world = World::on_map(map);
    let standing = map_buildings(map);
    let view = world.view(Team::Radiant);
    assert_eq!(
        view.units.len(),
        standing,
        "every building on the map is told, whosever it is"
    );
    let dire_fountain = map.fountains[1];
    assert!(
        view.units.iter().any(|u| u.pos == dire_fountain),
        "including the one across the map it has no eyes on"
    );
}

#[test]
fn a_unit_across_the_map_is_still_kept_from_a_side() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let far = world.spawn_unit(&MELEE_CREEP, Team::Dire, map.fountains[1]);
    world.settle();
    assert!(
        !world.can_see(Team::Radiant, far),
        "a creep is not a building"
    );
    let view = world.view(Team::Radiant);
    assert!(
        !view.units.iter().any(|u| u.id == wire_id(far)),
        "and the fog keeps it back"
    );
}

#[test]
fn a_unit_still_standing_never_reads_as_empty() {
    let mut world = World::new();
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::ZERO);
    world.settle();
    world.health.insert(
        creep,
        Health {
            hp: Fixed::from_ratio(1, 4),
        },
    );
    let view = world.view_full();
    let shown = view.units.first().expect("one unit").hp;
    assert_eq!(shown, 1, "a quarter of a point still shows as one");
}

#[test]
fn a_side_sees_what_stands_inside_its_sight() {
    let mut world = World::new();
    let watcher = hero_at(&mut world, Vec2::from_ints(1000, 1000));
    let near = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(1200, 1000));
    let far = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(9000, 9000));
    world.settle();
    world.step();
    let seen_near = world.visibility.get(near).expect("worked out this tick");
    assert!(seen_near.by(Team::Radiant), "close enough");
    assert!(seen_near.by(Team::Dire), "its own side always");
    let seen_far = world.visibility.get(far).expect("worked out this tick");
    assert!(!seen_far.by(Team::Radiant), "out of sight");
    let view = world.view(Team::Radiant);
    let ids: Vec<_> = view.units.iter().map(|u| u.id).collect();
    assert!(ids.contains(&wire_id(watcher)));
    assert!(ids.contains(&wire_id(near)));
    assert!(!ids.contains(&wire_id(far)), "fog holds it back");
}

#[test]
fn a_side_is_told_only_of_what_it_could_see() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    let far = Vec2::from_ints(9600, 12000);
    assert!(
        !world.can_see_point(Team::Radiant, far),
        "nothing of that side stands anywhere near"
    );
    assert_eq!(
        world.who_may_know(far, Team::Dire),
        EventVisibility::OneTeam(Team::Dire),
        "only the side party to it is told"
    );
    hero_at(&mut world, far);
    world.settle();
    assert!(world.can_see_point(Team::Radiant, far));
    assert_eq!(
        world.who_may_know(far, Team::Dire),
        EventVisibility::Everyone
    );
}

#[test]
fn a_ranged_attack_puts_a_missile_where_a_side_can_see_it() {
    let mut world = World::new();
    let archer = hero_at(&mut world, Vec2::from_ints(5000, 5000));
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5300, 5000));
    world.settle();
    world.fill_pools(archer);
    world.fill_pools(mark);
    let flew = (0..40).any(|_| {
        world.step();
        !world.view_full().projectiles.is_empty()
    });
    assert!(
        flew,
        "a ranged hero throws something the client is told about"
    );
    let view = world.view(Team::Radiant);
    assert!(
        !view.projectiles.is_empty(),
        "and its own side is told of it"
    );
}

#[test]
fn a_missile_names_who_let_it_go_and_whom_it_homes_on_while_the_side_sees_them() {
    // Dota's tracking missile shows whom it homes on and who let it go.
    let mut world = World::new();
    let archer = world.spawn_unit(&RANGED_CREEP, Team::Radiant, Vec2::from_ints(5000, 5000));
    let mark = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5300, 5000));
    world.settle();
    let thrown = (0..40).any(|_| {
        world.step();
        !world.view_full().projectiles.is_empty()
    });
    assert!(thrown, "the archer lets a missile go");
    // The other side's sight takes in the missile on the next tick.
    world.step();
    for side in [None, Some(Team::Radiant), Some(Team::Dire)] {
        let view = side.map_or_else(|| world.view_full(), |team| world.view(team));
        let shot = view.projectiles[0];
        assert_eq!(shot.source, Some(wire_id(archer)), "{side:?}");
        assert_eq!(shot.target, Some(wire_id(mark)), "{side:?}");
    }
    world.transform.get_mut(archer).expect("standing").pos = Vec2::from_ints(1000, 1000);
    world.settle();
    let view = world.view(Team::Dire);
    assert!(
        view.unit(wire_id(archer)).is_none(),
        "the Dire side lost sight of it"
    );
    let shot = view.projectiles[0];
    assert_eq!(
        shot.source, None,
        "so the missile does not say who let it go"
    );
    assert_eq!(
        shot.target,
        Some(wire_id(mark)),
        "but still whom it homes on"
    );
}

#[test]
fn a_missile_is_seen_from_the_tick_it_is_thrown() {
    let mut world = World::new();
    world.spawn_unit(&RANGED_CREEP, Team::Radiant, Vec2::from_ints(5000, 5000));
    world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5300, 5000));
    world.settle();
    let missile = (0..40)
        .find_map(|_| {
            world.step();
            world
                .entities
                .iter()
                .find(|e| world.projectile.get(*e).is_some())
        })
        .expect("the archer threw something");
    assert!(
        world
            .visibility
            .get(missile)
            .is_some_and(|s| s.by(Team::Radiant)),
        "its own side has it"
    );
}

/// A Dire archer that cannot walk and swings once every five seconds,
/// reaching far enough to strike down a cliff.
const CLIFF_ARCHER: bota_core::UnitDef = bota_core::UnitDef {
    max_hp: 8_000,
    attack_range: 700,
    attack_time: 5_000,
    move_speed: 0,
    ..RANGED_CREEP
};

#[test]
fn a_strike_from_high_ground_shows_the_striker_to_the_side_struck_for_two_seconds() {
    let mut world = World::new();
    let (low, _, high) = nearby_elevations(&world.ground);
    let archer = world.spawn_unit(&CLIFF_ARCHER, Team::Dire, high);
    let mark = world.spawn_unit(&UPHILL_TEST_TARGET, Team::Radiant, low);
    world.settle();
    world.fill_pools(mark);
    let shown = |world: &World| {
        let listed = world
            .view(Team::Radiant)
            .units
            .iter()
            .any(|unit| unit.id == wire_id(archer));
        assert_eq!(
            listed,
            world.can_see(Team::Radiant, archer),
            "the view tells exactly what the side sees"
        );
        listed
    };
    assert!(!shown(&world), "from below, high ground is fog");
    let struck = (0..60)
        .find(|_| {
            world.step();
            world.exposure.get(archer).is_some()
        })
        .map(|_| world.tick)
        .expect("the archer strikes within two seconds");
    assert!(
        !shown(&world),
        "the strike shows from the next sight pass on"
    );
    for _ in 0..rules::STRIKE_REVEAL_TICKS {
        world.step();
        assert!(
            shown(&world),
            "tick {}: still shown {} ticks after the strike",
            world.tick,
            world.tick - struck
        );
    }
    world.step();
    assert!(!shown(&world), "and gone after two seconds");
    assert!(
        world.can_see(Team::Dire, mark),
        "the striker's own sight is its own business"
    );
}

#[test]
fn the_dire_mid_tier_two_tower_does_not_see_into_the_camp_behind_its_trees() {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    // By day, when the tower sees furthest.
    world.tick = rules::PREGAME_TICKS;
    world.settle();
    let tower = map
        .dire_towers
        .iter()
        .find(|(lane, tier, _)| *lane == rules::LANE_MID && *tier == 2)
        .map(|(_, _, at)| *at)
        .expect("Dire has a mid tier-two tower");
    // neutralcamp_evil_5, in the woods north-west of the tower.
    let camp = CAMPS
        .iter()
        .position(|camp| camp.pos == Vec2::from_ints(10280, 11796))
        .expect("the camp is on the map");
    let home = CAMPS[camp].pos;
    let reach = rules::units(rules::TOWER_DAY_VISION);
    assert!(
        tower.within(home, reach),
        "the camp lies inside the tower's sight radius"
    );
    // Where its neutrals stand and idle: within two hundred of its spot.
    let inner = rules::units(200);
    for dy in (-200..=200).step_by(25) {
        for dx in (-200..=200).step_by(25) {
            let at = home + Vec2::from_ints(dx, dy);
            if at.within(home, inner) {
                assert!(
                    !world.can_see_point(Team::Dire, at),
                    "Dire sees {at:?} in the camp through the trees"
                );
            }
        }
    }
    // As far off down the open lane, the tower sees plainly.
    let lane = tower + Vec2::from_ints(-1211, -897);
    assert!(tower.within(lane, reach));
    assert!(
        world.can_see_point(Team::Dire, lane),
        "the open lane is in sight"
    );
}

/// A row of nine trees two cells apart on open low ground far off the demo
/// lane, the spacing most of the Dota forest stands at.
const TREE_ROW: [(i16, i16); 9] = {
    let mut row = [(0, 0); 9];
    let mut i = 0;
    while i < row.len() {
        row[i] = (2048 + 128 * i as i16, 2048);
        i += 1;
    }
    row
};

/// The demo map's ground with nothing but [`TREE_ROW`] for a forest.
static ROW_TERRAIN: crate::game::MapTerrain = crate::game::MapTerrain {
    trees: &TREE_ROW,
    ..crate::game::DEMO_TERRAIN
};

/// A world on the demo map and [`ROW_TERRAIN`] with a Radiant hero a quelling blade in hand
/// south of the row's middle tree, and that tree.
fn a_hero_below_the_row() -> (World, crate::game::Entity, Vec2) {
    let mut world = World::on_terrain(&MAPS[1], &ROW_TERRAIN);
    let middle = Vec2::from_ints(2560, 2048);
    let hero = seated_hero(
        &mut world,
        middle + Vec2::from_ints(0, -320),
        HeroId::SHADOW_FIEND,
    );
    hand_item(&mut world, hero, ItemId::QUELLING_BLADE, 0);
    world.settle();
    (world, hero, middle)
}

/// A Dire post that never walks.
const POST: bota_core::UnitDef = bota_core::UnitDef {
    move_speed: 0,
    ..MELEE_CREEP
};

#[test]
fn a_row_of_trees_two_cells_apart_is_a_closed_wall_to_sight() {
    let (mut world, hero, middle) = a_hero_below_the_row();
    let stand = world.transform.get(hero).expect("hero").pos;
    // Aimed across the row between two trunks, where single-cell trees
    // left a gap.
    let across = world.spawn_unit(&POST, Team::Dire, middle + Vec2::from_ints(192, 320));
    // The same distance along the row's own side, in the open.
    let beside = world.spawn_unit(&POST, Team::Dire, stand + Vec2::from_ints(-640, 0));
    world.settle();
    assert!(world.can_see(Team::Radiant, beside), "open ground is seen");
    assert!(
        !world.can_see(Team::Radiant, across),
        "the row shuts sight between its trunks"
    );
}

#[test]
fn a_felled_tree_opens_a_gap_in_the_row_and_its_regrowth_shuts_it() {
    let (mut world, hero, middle) = a_hero_below_the_row();
    let behind = world.spawn_unit(&POST, Team::Dire, middle + Vec2::from_ints(0, 320));
    world.settle();
    assert!(
        !world.can_see(Team::Radiant, behind),
        "the tree stands between"
    );
    assert!(
        world.use_item(hero, 0, Target::Pos(middle), &mut Vec::new()),
        "the blade reaches the tree"
    );
    world.step();
    assert!(
        world.can_see(Team::Radiant, behind),
        "with the tree down, the gap shows what stands behind it"
    );
    run(&mut world, rules::TREE_REGROW_TICKS);
    assert_eq!(world.trees.felled().count(), 0, "the tree grew back");
    assert!(!world.can_see(Team::Radiant, behind), "and shuts the gap");
}

#[test]
fn the_client_pictures_the_servers_sight_cell_for_cell() {
    use bota_client_core::{AckPolicy, ClientSession, MatchStatic, SessionOptions, SightGround};
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    // The forest changed: the trees round the camp beside the Dire mid
    // tier-two tower down, and one put up in the open by the tower.
    let camp = Vec2::from_ints(10280, 11796);
    for (index, at) in crate::game::tree_positions(map, crate::game::terrain_of(map))
        .into_iter()
        .enumerate()
    {
        if at.within(camp, rules::units(500)) {
            world
                .trees
                .fell(crate::game::Tree::Rooted(index), world.tick);
        }
    }
    let tower = Vec2::from_ints(11712, 11328);
    world
        .trees
        .plant(tower + Vec2::from_ints(-400, 0), world.tick + 100);
    world.lay_sight_block();
    let home = Vec2::from_ints(1500, 1500);
    seated_hero(&mut world, home, HeroId::SHADOW_FIEND);
    world.settle();
    let mut config = config();
    config.map = MapId(0);
    let mut session = ClientSession::spectator(
        MatchStatic::new(&config.info()).expect("a match"),
        SessionOptions {
            ack: AckPolicy::Never,
            ..SessionOptions::default()
        },
    );
    session
        .ingest_snapshot(world.view(Team::Dire), &mut ())
        .expect("a snapshot");
    let view = session.current().expect("a snapshot came");
    let ground = SightGround::lay(session.statics(), Some(view)).expect("the sight grid");
    assert!(ground.fits(view));
    let n = rules::GRID_CELLS;
    let mut lit = vec![false; n * n];
    ground.lit(view, Team::Dire, &mut lit);
    let reach = rules::TOWER_DAY_VISION / rules::GRID_CELL_SIZE + 1;
    let (tx, ty) = CellGrid::cell_of(tower).expect("on the map");
    let mut differ = Vec::new();
    let mut seen = 0;
    for cy in ty - reach as usize..=ty + reach as usize {
        for cx in tx - reach as usize..=tx + reach as usize {
            let server = world.can_see_point(Team::Dire, CellGrid::cell_center((cx, cy)));
            seen += usize::from(server);
            if server != lit[cy * n + cx] {
                differ.push((cx, cy, server));
            }
        }
    }
    assert!(seen > 1000, "the tower sees a wide field");
    assert!(differ.is_empty(), "the client differs at {differ:?}");
}

/// Whether a side is told of a unit, and how far its own hero sees, as the
/// side's view of the current tick says.
fn told_and_hero_sight(world: &World, mark: Entity) -> (bool, i32) {
    let view = world.view(Team::Radiant);
    let hero = view
        .units
        .iter()
        .find(|u| u.kind == UnitKind::Hero)
        .expect("its own hero is always told");
    let told = view.units.iter().any(|u| u.id == wire_id(mark));
    (told, hero.vision_radius.to_int())
}

#[test]
fn a_heros_sight_is_night_vision_through_the_pregame_and_the_night_and_day_vision_by_day() {
    let between = (rules::HERO_DAY_VISION + rules::HERO_NIGHT_VISION) / 2;
    let mut world = World::new();
    hero_at(&mut world, Vec2::from_ints(1000, 1000));
    let mark = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        Vec2::from_ints(1000 + between, 1000),
    );
    world.settle();
    let mut seen = Vec::new();
    while world.tick < rules::PREGAME_TICKS + 2 * rules::HALF_DAY_TICKS {
        let (told, sight) = told_and_hero_sight(&world, mark);
        let night = bota_core::time_of_day(world.tick).night;
        let sees = if night {
            rules::HERO_NIGHT_VISION
        } else {
            rules::HERO_DAY_VISION
        };
        assert_eq!((told, sight), (!night, sees), "on tick {}", world.tick);
        if seen.last() != Some(&night) {
            seen.push(night);
        }
        world.step();
    }
    assert_eq!(
        seen,
        [true, false, true],
        "the pregame is night, the horn breaks the first day, the night falls five minutes on"
    );
    let dawn = rules::PREGAME_TICKS + 2 * rules::HALF_DAY_TICKS;
    assert_eq!(world.tick, dawn);
    assert!(
        told_and_hero_sight(&world, mark).0,
        "and the second dawn opens the fog over it again"
    );
}

#[test]
fn a_tier_one_tower_sees_less_far_by_night_than_the_towers_behind_it() {
    let mut world = World::on_map(map_of(MapId(0)));
    world.tick = rules::PREGAME_TICKS + rules::HALF_DAY_TICKS;
    world.settle();
    let view = world.view_full();
    let sight_of = |tier: u8| {
        let tower = world
            .entities
            .iter()
            .find(|e| world.tier.get(*e).is_some_and(|t| t.0 == tier))
            .expect("every tier stands");
        view.units
            .iter()
            .find(|u| u.id == wire_id(tower))
            .expect("every building is told")
            .vision_radius
            .to_int()
    };
    assert_eq!(sight_of(1), 800);
    for tier in 2..=4 {
        assert_eq!(sight_of(tier), 1100, "tier {tier}");
    }
}
