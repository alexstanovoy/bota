//! Who sees what, and what each side is told.

use bota_core::{CAMPS, CellGrid, MAPS, MELEE_CREEP, RANGED_CREEP, map_of, rules};

use crate::game::{Entity, Event, Health, World, wire_id};
use bota_proto::{
    EntityId, EventKind, Fixed, HeroId, ItemId, MapId, ProjectileModel, Target, Team, UnitKind,
    Vec2,
};

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
fn a_blow_on_a_hidden_ward_is_told_only_to_a_side_whose_true_sight_finds_it() {
    let mut world = World::new();
    let spot = Vec2::from_ints(3000, 3000);
    hero_at(&mut world, spot + Vec2::from_ints(300, 0));
    let ward = world.spawn_ward(
        &bota_core::OBSERVER_WARD,
        (Team::Dire, spot),
        1_000,
        (None, None),
    );
    world.settle();
    assert!(
        world.can_see_point(Team::Radiant, spot),
        "the hero sees the ground the ward stands on"
    );
    let told_of_blow = |world: &mut World, team| {
        // A swing: a ward takes nothing from what is no attack.
        world.push_hit(None, ward, 10, bota_proto::DamageKind::Physical, None);
        let blow = world.hits.back_mut().expect("just laid");
        blow.attack = true;
        blow.striker = Some(bota_core::Striker::Other);
        let events = world.step();
        crate::game_loop::visible_events(&events, Some(team))
            .iter()
            .any(|kind| matches!(kind, bota_proto::EventKind::Damaged { target, .. } if *target == wire_id(ward)))
    };
    assert!(
        !told_of_blow(&mut world, Team::Radiant),
        "seeing the ground is not seeing the ward"
    );
    assert!(told_of_blow(&mut world, Team::Dire), "its own side is told");
    world.spawn_ward(
        &bota_core::SENTRY_WARD,
        (Team::Radiant, spot + Vec2::from_ints(200, 0)),
        1_000,
        (None, None),
    );
    world.settle();
    assert!(
        told_of_blow(&mut world, Team::Radiant),
        "true sight finds it, and the blow is told"
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
fn a_fogged_attackers_missile_shows_its_look_but_not_who_let_it_go() {
    // Dota draws a missile out of the fog with its own particle: the
    // projectile carries its look, never its thrower.
    let mut world = World::new();
    let archer = world.spawn_unit(&RANGED_CREEP, Team::Radiant, Vec2::from_ints(5000, 5000));
    world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5300, 5000));
    world.settle();
    let thrown = (0..40).any(|_| {
        world.step();
        !world.view_full().projectiles.is_empty()
    });
    assert!(thrown, "the archer lets a missile go");
    world.step();
    world.transform.get_mut(archer).expect("standing").pos = Vec2::from_ints(1000, 1000);
    world.settle();
    for side in [None, Some(Team::Radiant), Some(Team::Dire)] {
        let view = side.map_or_else(|| world.view_full(), |team| world.view(team));
        let shot = view.projectiles[0];
        assert_eq!(shot.model, Some(ProjectileModel::RangedCreep), "{side:?}");
        assert_eq!(shot.team, Team::Radiant, "{side:?}");
    }
    let view = world.view(Team::Dire);
    assert!(
        view.unit(wire_id(archer)).is_none(),
        "the Dire side lost sight of the archer"
    );
    assert_eq!(
        view.projectiles[0].source, None,
        "so its missile shows how it looks but not who let it go"
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

/// The view of everything says which sides see each unit, missile and
/// ground item exactly as the sides' own views hold them, through a lane
/// fight in the fog; a side's view never says it.
#[test]
fn the_whole_map_says_which_sides_see_what_as_their_views_hold_it() {
    let config = super::local::mid_config();
    let mut world = World::for_match(&config, config.rng());
    let (mut told, mut unseen) = (0, 0);
    for _ in 0..40 {
        run(&mut world, 60);
        let full = world.view_full();
        let sides = [Team::Radiant, Team::Dire].map(|team| world.view(team));
        for side in &sides {
            assert!(side.units.iter().all(|unit| unit.seen_by.is_none()));
            assert!(side.projectiles.iter().all(|shot| shot.seen_by.is_none()));
            assert!(side.loot.iter().all(|item| item.seen_by.is_none()));
        }
        for (team, side) in [Team::Radiant, Team::Dire].into_iter().zip(&sides) {
            for unit in &full.units {
                let seen = unit.seen_by.expect("the whole map says").by(team);
                assert_eq!(seen, side.units.iter().any(|held| held.id == unit.id));
                told += 1;
                unseen += usize::from(!seen);
            }
            for shot in &full.projectiles {
                let seen = shot.seen_by.expect("the whole map says").by(team);
                assert_eq!(seen, side.projectiles.iter().any(|held| held.id == shot.id));
            }
            for item in &full.loot {
                let seen = item.seen_by.expect("the whole map says").by(team);
                assert_eq!(seen, side.loot.iter().any(|held| held.id == item.id));
            }
        }
    }
    assert!(
        unseen > 0 && unseen < told,
        "the fog hid some units, not all"
    );
}

#[test]
fn a_hero_that_blinks_out_of_sight_is_not_shown_where_it_landed() {
    let mut world = World::new();
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(1000, 1000));
    let blinker = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(1500, 1000));
    hand_item(&mut world, blinker, ItemId::BLINK_DAGGER, 0);
    world.settle();
    assert!(world.view(Team::Radiant).unit(wire_id(blinker)).is_some());
    let from = world.pos(blinker);
    let landing = Vec2::from_ints(2600, 1000);
    let order = bota_proto::Order::Use {
        slot: bota_proto::ItemSlot(0),
        target: Target::Pos(landing),
    };
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(1),
        unit: None,
        order,
        queue: false,
        seq: 0,
    }]);
    for _ in 0..30 {
        if world.pos(blinker) != from {
            break;
        }
        world.step();
    }
    assert_eq!(world.pos(blinker), Some(landing), "it blinked");
    assert!(
        world.view(Team::Radiant).unit(wire_id(blinker)).is_none(),
        "the night hides the landing from the side it left on the very tick it lands"
    );
}

/// Razes from Shadow Fiend on high ground, unseen from below, until what
/// the raze struck falls; every event of those ticks.
fn raze_from_the_fog(world: &mut World, fiend: Entity, slot: u8, mark: Entity) -> Vec<Event> {
    assert!(
        !world.can_see(Team::Radiant, fiend),
        "from below, the cliff is fog"
    );
    let mut events = cast(world, slot, Target::None);
    for _ in 0..60 {
        if !world.alive(mark) {
            return events;
        }
        events.extend(world.step());
    }
    panic!("the raze never brought its mark down");
}

/// What a side is told of a blow on a unit and of its fall: the blow's
/// dealer, then the killer.
fn dealers_told(
    events: &[Event],
    team: Team,
    mark: Entity,
) -> (Option<EntityId>, Option<EntityId>) {
    let told = crate::game_loop::visible_events(events, Some(team));
    let source = told.iter().find_map(|kind| match kind {
        EventKind::Damaged { source, target, .. } if *target == wire_id(mark) => Some(*source),
        _ => None,
    });
    let killer = told.iter().find_map(|kind| match kind {
        EventKind::Died { killer, unit, .. } if *unit == wire_id(mark) => Some(*killer),
        _ => None,
    });
    (
        source.expect("the blow is told"),
        killer.expect("the fall is told"),
    )
}

#[test]
fn a_creep_razed_from_the_fog_never_names_the_fiend_to_its_side() {
    let (mut world, fiend, spot, slot) = fiend_across_a_cliff(false);
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, spot);
    world.settle();
    world.health.insert(creep, health(1));
    let events = raze_from_the_fog(&mut world, fiend, slot, creep);
    assert_eq!(
        dealers_told(&events, Team::Radiant, creep),
        (None, None),
        "a side that does not see the dealer is told nobody dealt it"
    );
    assert_eq!(
        dealers_told(&events, Team::Dire, creep),
        (Some(wire_id(fiend)), Some(wire_id(fiend))),
        "the dealer's own side knows its own"
    );
}

#[test]
fn a_hero_razed_from_the_fog_learns_its_killer_from_the_kill_feed_alone() {
    let (mut world, fiend, spot, slot) = fiend_across_a_cliff(false);
    let hero = seat_hero(&mut world, 1, Team::Radiant, spot);
    world.settle();
    world.health.insert(hero, health(1));
    let events = raze_from_the_fog(&mut world, fiend, slot, hero);
    assert_eq!(
        dealers_told(&events, Team::Radiant, hero),
        (None, Some(wire_id(fiend))),
        "the blow names nobody; the hero's fall is announced with its killer"
    );
}

#[test]
fn a_level_gained_out_of_sight_is_told_to_its_own_side_alone() {
    let mut world = World::new();
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(1000, 1000));
    let far = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(9000, 9000));
    world.settle();
    let level_ups = |events: &[Event], team| {
        crate::game_loop::visible_events(events, Some(team))
            .iter()
            .filter(|kind| matches!(kind, EventKind::LevelUp { unit, .. } if *unit == wire_id(far)))
            .count()
    };
    let mut events = Vec::new();
    world.grant_xp(1, rules::XP_THRESHOLDS[1], &mut events);
    assert_eq!(level_ups(&events, Team::Dire), 1, "its own side is told");
    assert_eq!(level_ups(&events, Team::Radiant), 0, "the fog keeps it");
    if let Some(at) = world.transform.get_mut(far) {
        at.pos = Vec2::from_ints(1300, 1000);
    }
    world.settle();
    let mut events = Vec::new();
    world.grant_xp(
        1,
        rules::XP_THRESHOLDS[2] - rules::XP_THRESHOLDS[1],
        &mut events,
    );
    assert_eq!(
        level_ups(&events, Team::Radiant),
        1,
        "a level seen gained is told"
    );
}

/// One of the big map's trees with open ground beside it that no Dire
/// building sees: its index and where it stands.
fn a_tree_out_of_dire_sight(world: &World) -> (u32, Vec2) {
    world
        .trees
        .spots()
        .iter()
        .enumerate()
        .find(|(_, at)| {
            world.clearance.stands_clear(**at + Vec2::from_ints(120, 0))
                && world.clearance.stands_clear(**at + Vec2::from_ints(240, 0))
                && !world.can_see_point(Team::Dire, **at + Vec2::from_ints(240, 0))
        })
        .map(|(index, at)| (index as u32, *at))
        .expect("some tree stands in the fog")
}

/// Seat 0 uses the item in its first slot on a spot, and the world runs
/// until the forest has changed.
fn use_on_the_forest(world: &mut World, at: Vec2) -> Vec<Event> {
    let before = (world.trees.felled().count(), world.trees.planted().len());
    let order = bota_proto::Order::Use {
        slot: bota_proto::ItemSlot(0),
        target: Target::Pos(at),
    };
    let mut events = seat_order(world, order);
    for _ in 0..60 {
        if (world.trees.felled().count(), world.trees.planted().len()) != before {
            return events;
        }
        events = world.step();
    }
    panic!("the item took the forest");
}

/// The tree events of a tick a side is told: where each tree fell, by
/// whom, and where each grew back.
fn trees_told(events: &[Event], team: Team) -> (Vec<(Vec2, Option<EntityId>)>, Vec<Vec2>) {
    let (mut felled, mut grown) = (Vec::new(), Vec::new());
    for event in events.iter().filter(|event| event.visible_to.tells(team)) {
        match event.kind {
            EventKind::TreeFelled { at, by } => {
                felled.push((at, by.filter(|_| event.source_told_to.tells(team))));
            }
            EventKind::TreeGrew { at } => grown.push(at),
            _ => {}
        }
    }
    (felled, grown)
}

/// Puts a unit somewhere else and works sight out afresh.
fn put(world: &mut World, unit: Entity, at: Vec2) {
    if let Some(transform) = world.transform.get_mut(unit) {
        transform.pos = at;
    }
    world.settle();
}

/// A Tango eats its tree: the tree is consumed (Liquipedia *Tango*,
/// Devour: "Consumes a tree"), told as it falls to the side that sees it,
/// and grows back after Dota's three minutes, though not while a unit stands
/// within 150 of its spot (Liquipedia *Trees*, Respawn Time), told as it
/// grows to who sees it. A side that sees neither keeps the forest as it last
/// saw it until it looks (since 2014 Dota shows no tree falling or growing in
/// the fog).
#[test]
fn a_tango_eats_its_tree_which_grows_back_in_three_minutes_once_nobody_stands_by() {
    let mut world = World::on_map(map_of(MapId(0)));
    let (index, tree) = a_tree_out_of_dire_sight(&world);
    let eater = seat_hero(&mut world, 0, Team::Radiant, tree + Vec2::from_ints(120, 0));
    let looker = seat_hero(&mut world, 1, Team::Dire, map_of(MapId(0)).fountains[1]);
    hand_item(&mut world, eater, ItemId::TANGO, 3);
    world.settle();
    let felled = |world: &World, side: Option<Team>| {
        let view = side.map_or_else(|| world.view_full(), |team| world.view(team));
        view.felled_trees.contains(&index)
    };
    let events = use_on_the_forest(&mut world, tree);
    let eaten_at = world.tick;
    assert!(felled(&world, None), "the tree is down");
    let by = Some(wire_id(eater));
    assert_eq!(
        trees_told(&events, Team::Radiant),
        (vec![(tree, by)], vec![]),
        "the eater's side sees it fall"
    );
    assert_eq!(
        trees_told(&events, Team::Dire),
        (vec![], vec![]),
        "the other side is told nothing"
    );
    assert!(
        felled(&world, Some(Team::Radiant)),
        "the eater's side has it down"
    );
    assert!(
        !felled(&world, Some(Team::Dire)),
        "the other side still has it standing"
    );
    put(&mut world, looker, tree + Vec2::from_ints(240, 0));
    assert!(felled(&world, Some(Team::Dire)), "until it looks");
    put(&mut world, looker, map_of(MapId(0)).fountains[1]);
    let aside = tree + Vec2::from_ints(rules::TREE_REGROW_CLEAR + 40, 0);
    put(&mut world, eater, aside);
    // Dota's three minutes, not whatever the rules hold.
    world.tick = eaten_at + 3 * 60 * rules::TICKS_PER_SECOND - 3;
    for _ in 0..2 {
        let events = world.step();
        assert!(
            felled(&world, None),
            "with nobody by it, it is still down a tick short of three minutes"
        );
        assert_eq!(trees_told(&events, Team::Radiant), (vec![], vec![]));
    }
    put(&mut world, eater, tree + Vec2::from_ints(120, 0));
    for _ in 0..3 {
        let events = world.step();
        assert!(
            felled(&world, None),
            "the eater stands by it: it does not grow under him"
        );
        assert_eq!(trees_told(&events, Team::Radiant), (vec![], vec![]));
    }
    put(
        &mut world,
        eater,
        tree + Vec2::from_ints(rules::TREE_REGROW_CLEAR + 40, 0),
    );
    let events = world.step();
    assert!(!felled(&world, None), "with nobody by it, it grew back");
    assert_eq!(
        trees_told(&events, Team::Radiant),
        (vec![], vec![tree]),
        "where the eater sees it"
    );
    assert_eq!(trees_told(&events, Team::Dire), (vec![], vec![]));
    assert!(!felled(&world, Some(Team::Radiant)), "and has it standing");
    assert!(
        felled(&world, Some(Team::Dire)),
        "the other side, gone, still has it down"
    );
}

#[test]
fn a_tree_planted_in_the_fog_shows_only_once_the_other_side_looks() {
    let mut world = World::on_map(map_of(MapId(0)));
    let (_, tree) = a_tree_out_of_dire_sight(&world);
    let planter = seat_hero(&mut world, 0, Team::Radiant, tree + Vec2::from_ints(120, 0));
    let looker = seat_hero(&mut world, 1, Team::Dire, map_of(MapId(0)).fountains[1]);
    hand_item(&mut world, planter, ItemId::IRON_BRANCH, 1);
    world.settle();
    let spot = tree + Vec2::from_ints(240, 0);
    let planted = |world: &World, side: Option<Team>| {
        let view = side.map_or_else(|| world.view_full(), |team| world.view(team));
        view.planted_trees.contains(&spot)
    };
    use_on_the_forest(&mut world, spot);
    assert!(planted(&world, None), "a tree stands");
    assert!(planted(&world, Some(Team::Radiant)));
    assert!(!planted(&world, Some(Team::Dire)), "nobody of Dire saw it");
    put(&mut world, looker, spot + Vec2::from_ints(120, 0));
    assert!(planted(&world, Some(Team::Dire)), "until one looks");
    put(&mut world, looker, map_of(MapId(0)).fountains[1]);
    run(&mut world, planted_tree_ticks() + 1);
    assert!(!planted(&world, None), "it went in its time");
    assert!(
        !planted(&world, Some(Team::Radiant)),
        "where the planter saw"
    );
    assert!(
        planted(&world, Some(Team::Dire)),
        "while the other side, gone, still has it standing"
    );
}

#[test]
fn the_scoreboard_shows_an_enemy_hero_as_it_was_last_seen() {
    let mut world = World::new();
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(1000, 1000));
    let enemy = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(1300, 1000));
    world.settle();
    let scored = |world: &World, side: Option<Team>| {
        let view = side.map_or_else(|| world.view_full(), |team| world.view(team));
        let player = view
            .player(bota_proto::SlotId(1))
            .expect("every seat is scored");
        let score = player.creep_score.map(|score| score.last_hits);
        (player.level, player.xp, player.seen_at, score)
    };
    world.seats[1].last_hits = 4;
    world.grant_xp(1, rules::XP_THRESHOLDS[1], &mut Vec::new());
    world.step();
    let seen_on = world.tick;
    let seen = (2, rules::XP_THRESHOLDS[1]);
    assert_eq!(
        scored(&world, Some(Team::Radiant)),
        (seen.0, seen.1, Some(seen_on), None),
        "seen now, and its creep score withheld"
    );
    put(&mut world, enemy, Vec2::from_ints(9000, 9000));
    world.grant_xp(1, rules::XP_THRESHOLDS[3], &mut Vec::new());
    world.step();
    assert_eq!(
        scored(&world, Some(Team::Radiant)),
        (seen.0, seen.1, Some(seen_on), None),
        "out of sight, the level and experience as of the tick last seen"
    );
    let now = (4, rules::XP_THRESHOLDS[1] + rules::XP_THRESHOLDS[3]);
    let tick = Some(world.tick);
    assert_eq!(
        scored(&world, Some(Team::Dire)),
        (now.0, now.1, tick, Some(4)),
        "its own side"
    );
    assert_eq!(
        scored(&world, None),
        (now.0, now.1, tick, Some(4)),
        "a view of everything"
    );
    put(&mut world, enemy, Vec2::from_ints(1300, 1000));
    assert_eq!(
        scored(&world, Some(Team::Radiant)),
        (now.0, now.1, tick, None),
        "seen again, as it is"
    );
}

#[test]
fn a_hero_never_seen_is_scored_as_a_match_starts_it_and_says_so() {
    let mut world = World::new();
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(1000, 1000));
    seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(9000, 9000));
    world.settle();
    world.grant_xp(1, rules::XP_THRESHOLDS[2], &mut Vec::new());
    world.step();
    let player = world
        .view(Team::Radiant)
        .player(bota_proto::SlotId(1))
        .cloned()
        .expect("scored");
    assert_eq!(
        (player.level, player.xp, player.seen_at),
        (1, 0, None),
        "never seen: no tick to be as of"
    );
}

#[test]
fn an_enemy_hero_in_sight_keeps_its_unspent_points_and_its_sale_marks_to_itself() {
    let mut world = World::new();
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(1000, 1000));
    let enemy = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(1300, 1000));
    world.grant_xp(1, rules::XP_THRESHOLDS[1], &mut Vec::new());
    if let Some(bag) = world.inventory.get_mut(enemy) {
        bag.slots[0] = Some(crate::game::ItemStack {
            for_sale: true,
            ..fresh_stack(ItemId::IRON_BRANCH, 0)
        });
    }
    world.step();
    let plans = |side: Option<Team>| {
        let unit = shown(&world, side, enemy);
        let points = unit
            .abilities
            .iter()
            .find_map(|ability| ability.learnable.filter(|p| *p));
        let sale = unit
            .items
            .iter()
            .flatten()
            .find_map(|item| item.selling.filter(|s| *s));
        let withheld = unit
            .abilities
            .iter()
            .all(|ability| ability.learnable.is_none())
            && unit
                .items
                .iter()
                .flatten()
                .all(|item| item.selling.is_none());
        (points, sale, withheld)
    };
    assert_eq!(
        plans(Some(Team::Dire)),
        (Some(true), Some(true), false),
        "its own side knows"
    );
    assert_eq!(
        plans(None),
        (Some(true), Some(true), false),
        "and a view of everything"
    );
    assert_eq!(
        plans(Some(Team::Radiant)),
        (None, None, true),
        "the other side sees its abilities and items, and is told nothing of its plans for them"
    );
}

#[test]
fn a_hero_that_blinks_into_sight_is_shown_and_may_be_named_on_that_view() {
    let mut world = World::new();
    seat_hero(&mut world, 0, Team::Radiant, Vec2::from_ints(1000, 1000));
    let blinker = seat_hero(&mut world, 1, Team::Dire, Vec2::from_ints(2600, 1000));
    hand_item(&mut world, blinker, ItemId::BLINK_DAGGER, 0);
    world.settle();
    assert!(world.view(Team::Radiant).unit(wire_id(blinker)).is_none());
    let from = world.pos(blinker);
    let landing = Vec2::from_ints(1500, 1000);
    let order = bota_proto::Order::Use {
        slot: bota_proto::ItemSlot(0),
        target: Target::Pos(landing),
    };
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(1),
        unit: None,
        order,
        queue: false,
        seq: 0,
    }]);
    for _ in 0..30 {
        if world.pos(blinker) != from {
            break;
        }
        world.step();
    }
    assert_eq!(world.pos(blinker), Some(landing), "it blinked");
    assert!(
        world.view(Team::Radiant).unit(wire_id(blinker)).is_some(),
        "the view of the tick it lands shows it where it landed"
    );
    let attack = bota_proto::Order::Attack {
        target: Target::Unit(wire_id(blinker)),
    };
    assert_eq!(
        world.validate_order(bota_proto::SlotId(0), None, &attack),
        Ok(()),
        "and an order given on that view may name it"
    );
}

#[test]
fn a_missile_let_go_where_a_side_has_sight_is_shown_on_that_very_tick() {
    let mut world = World::new();
    hero_at(&mut world, Vec2::from_ints(5000, 5000));
    world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5100, 5000));
    let archer = world.spawn_unit(&RANGED_CREEP, Team::Dire, Vec2::from_ints(5500, 5000));
    world.settle();
    world.fill_pools(archer);
    let missile = (0..90)
        .find_map(|_| {
            world.step();
            world.entities.iter().find(|entity| {
                world.projectile.get(*entity).is_some()
                    && world.team.get(*entity) == Some(&Team::Dire)
            })
        })
        .expect("the archer lets a missile go");
    assert!(
        world
            .view(Team::Radiant)
            .projectiles
            .iter()
            .any(|shown| shown.id == wire_id(missile)),
        "the side whose sight it flies in is shown it as it is let go"
    );
}

/// What Radiant is told of who brought one of its buildings down, a Dire
/// unit standing out of its sight by Dire's fountain.
fn killer_told_of_a_fall(kind: UnitKind, killer_is_hero: bool) -> Option<EntityId> {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    let far = map.fountains[1];
    let killer = if killer_is_hero {
        seat_hero(&mut world, 1, Team::Dire, far)
    } else {
        world.spawn_unit(&MELEE_CREEP, Team::Dire, far)
    };
    world.settle();
    assert!(
        !world.can_see(Team::Radiant, killer),
        "the killer is out of sight"
    );
    let building = world
        .entities
        .iter()
        .find(|e| world.kind.get(*e) == Some(&kind) && world.team.get(*e) == Some(&Team::Radiant))
        .expect("Radiant has one");
    let mut events = Vec::new();
    world.bury(vec![(building, Some(killer))], &mut events);
    crate::game_loop::visible_events(&events, Some(Team::Radiant))
        .iter()
        .find_map(|told| match told {
            EventKind::Died { unit, killer, .. } if *unit == wire_id(building) => Some(*killer),
            _ => None,
        })
        .expect("a building's fall is told to both sides")
}

#[test]
fn the_feed_names_the_hero_that_razed_a_barracks_or_a_tower_and_no_other_killer() {
    assert!(
        killer_told_of_a_fall(UnitKind::Barracks, true).is_some(),
        "\"Barracks has been destroyed by\" a player"
    );
    assert!(
        killer_told_of_a_fall(UnitKind::Tower, true).is_some(),
        "\"destroyed a tower\""
    );
    assert_eq!(
        killer_told_of_a_fall(UnitKind::Tower, false),
        None,
        "\"a tower was destroyed by The Dire\": a creep is not named"
    );
}
