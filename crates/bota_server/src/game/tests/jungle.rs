//! Neutral camps.

use bota_core::{
    CAMPS, COURIER, MELEE_CREEP, NeutralAbility, NeutralKind, OBSERVER_WARD, SENTRY_WARD,
    SIEGE_CREEP, UnitDef, map_of, rules,
};

use crate::game::World;
use bota_proto::{DamageKind, EffectId, Fixed, ItemId, MapId, Team, Vec2};

use super::support::*;

#[test]
fn camps_fill_on_the_minute_and_stay_full() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_NEUTRAL_TICK {
        world.step();
    }
    let neutrals = |world: &World| {
        world
            .entities
            .iter()
            .filter(|e| world.camp_home.get(*e).is_some())
            .count()
    };
    let filled = neutrals(&world);
    assert!(filled > 0, "the jungle put something out");
    run(&mut world, rules::NEUTRAL_SPAWN_PERIOD_TICKS);
    assert_eq!(
        neutrals(&world),
        filled,
        "a full camp puts out nothing more"
    );
}

#[test]
fn a_neutral_led_too_far_gives_up_and_walks_home() {
    let map = map_of(MapId(1));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_NEUTRAL_TICK {
        world.step();
    }
    let beast = world
        .entities
        .iter()
        .find(|e| world.camp_home.get(*e).is_some())
        .expect("the jungle is full");
    let home = world.camp_home.get(beast).expect("it has one").home;
    // Carried well past its guard distance and held there.
    let away = home + Vec2::from_ints(1200, 0);
    if let Some(transform) = world.transform.get_mut(beast) {
        transform.pos = away;
    }
    for _ in 0..rules::NEUTRAL_AGGRO_WINDOW + 2 {
        world.step();
        if let Some(transform) = world.transform.get_mut(beast) {
            transform.pos = away;
        }
    }
    assert!(
        world.neutral_ai.get(beast).is_some_and(|ai| ai.going_home),
        "its patience ran out"
    );
    assert!(
        world.target.get(beast).is_none(),
        "and it takes nothing on while it walks back"
    );
    // Home again, it stands, and is led less far the next time.
    if let Some(transform) = world.transform.get_mut(beast) {
        transform.pos = home;
    }
    world.step();
    let ai = world.neutral_ai.get(beast).copied().expect("it has a mind");
    assert!(!ai.going_home, "back home it stands again");
    assert_eq!(
        ai.next_window,
        rules::NEUTRAL_SHORT_WINDOW,
        "and its patience is short from now on"
    );
}

/// What the map's first dry camp holds after its first fill, with one thing
/// put into the world beforehand.
fn first_fill_with(put: impl FnOnce(&mut World)) -> usize {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    put(&mut world);
    world.settle();
    while world.tick < rules::FIRST_NEUTRAL_TICK + 1 {
        world.step();
    }
    beasts_of(&world, CAMP)
}

/// A way to put a unit at a point.
type Blocker = fn(&mut World, Vec2);

/// The dry camp the box tests use.
const CAMP: usize = 1;

fn beasts_of(world: &World, camp: usize) -> usize {
    world
        .entities
        .iter()
        .filter(|e| {
            world
                .camp_home
                .get(*e)
                .is_some_and(|h| usize::from(h.camp) == camp)
        })
        .count()
}

fn inside() -> Vec2 {
    let b = CAMPS[CAMP].spawn_box;
    Vec2::from_ints(
        (b.min.x.to_int() + b.max.x.to_int()) / 2,
        (b.min.y.to_int() + b.max.y.to_int()) / 2,
    )
}

#[test]
fn an_empty_box_fills_and_what_stands_in_it_keeps_it_empty() {
    assert!(first_fill_with(|_| {}) > 0, "an empty box fills");
    let blockers: [(&str, Blocker); 6] = [
        ("an observer ward", |w, at| {
            w.spawn_unit(&OBSERVER_WARD, Team::Radiant, at);
        }),
        ("a sentry ward", |w, at| {
            w.spawn_unit(&SENTRY_WARD, Team::Dire, at);
        }),
        ("a hero", |w, at| {
            hero_at(w, at);
        }),
        ("a lane creep", |w, at| {
            w.spawn_unit(&MELEE_CREEP, Team::Dire, at);
        }),
        ("a siege creep that does not move", |w, at| {
            let siege = w.spawn_unit(&SIEGE_CREEP, Team::Dire, at);
            w.settle();
            w.stats.get_mut(siege).expect("stats").move_speed = Fixed::ZERO;
        }),
        ("a rooted hero", |w, at| {
            let hero = hero_at(w, at);
            w.settle();
            w.stats.get_mut(hero).expect("stats").move_speed = Fixed::ZERO;
        }),
    ];
    for (name, put) in blockers {
        let beasts = first_fill_with(|world| put(world, inside()));
        assert_eq!(beasts, 0, "{name} in the box keeps the camp empty");
    }
}

#[test]
fn a_dead_hero_a_courier_and_loot_do_not_block() {
    let dead = first_fill_with(|world| {
        let hero = hero_at(world, inside());
        world.settle();
        world.health.get_mut(hero).expect("health").hp = Fixed::ZERO;
    });
    assert!(dead > 0, "a dead hero does not keep the camp empty");
    let courier = first_fill_with(|world| {
        world.spawn_unit(&COURIER, Team::Radiant, inside());
    });
    assert!(courier > 0, "a courier does not");
    let loot = first_fill_with(|world| {
        world.lay_loot(a_stack_of(ItemId::IRON_BRANCH, 0), inside());
    });
    assert!(loot > 0, "an item on the ground does not");
}

#[test]
fn a_ward_blocks_to_the_edge_of_the_box_and_not_past_it() {
    let b = CAMPS[CAMP].spawn_box;
    let mid_y = (b.min.y.to_int() + b.max.y.to_int()) / 2;
    let cases = [
        (b.max.x.to_int(), mid_y, true),
        (b.max.x.to_int() + 1, mid_y, false),
        (b.min.x.to_int(), mid_y, true),
        (b.min.x.to_int() - 1, mid_y, false),
        (b.min.x.to_int() + 1, b.max.y.to_int(), true),
        (b.min.x.to_int() + 1, b.max.y.to_int() + 1, false),
        (b.min.x.to_int() + 1, b.min.y.to_int(), true),
        (b.min.x.to_int() + 1, b.min.y.to_int() - 1, false),
    ];
    for (x, y, blocks) in cases {
        let beasts = first_fill_with(|world| {
            world.spawn_unit(&OBSERVER_WARD, Team::Radiant, Vec2::from_ints(x, y));
        });
        assert_eq!(
            beasts == 0,
            blocks,
            "a ward at ({x}, {y}) against the box {b:?}"
        );
    }
}

#[test]
fn pulling_a_camp_out_of_its_box_before_the_minute_stacks_it() {
    let map = map_of(MapId(0));
    let mut world = World::on_map(map);
    while world.tick < rules::FIRST_NEUTRAL_TICK + 1 {
        world.step();
    }
    let first = beasts_of(&world, CAMP);
    assert!(first > 0);
    let b = CAMPS[CAMP].spawn_box;
    let out = Vec2::from_ints(b.max.x.to_int() + 200, b.max.y.to_int() + 200);
    let mine: Vec<_> = world
        .entities
        .iter()
        .filter(|e| {
            world
                .camp_home
                .get(*e)
                .is_some_and(|h| usize::from(h.camp) == CAMP)
        })
        .collect();
    // Held in the box, one stays and the camp stays closed.
    for beast in &mine[1..] {
        world.transform.get_mut(*beast).expect("stands").pos = out;
    }
    while world.tick < rules::FIRST_NEUTRAL_TICK + rules::NEUTRAL_SPAWN_PERIOD_TICKS {
        world.step();
        for beast in &mine[1..] {
            world.transform.get_mut(*beast).expect("stands").pos = out;
        }
    }
    assert_eq!(beasts_of(&world, CAMP), first, "one left inside blocks it");
    // All of them out, the next minute puts a second camp beside them.
    world.transform.get_mut(mine[0]).expect("stands").pos = out;
    while world.tick < rules::FIRST_NEUTRAL_TICK + 2 * rules::NEUTRAL_SPAWN_PERIOD_TICKS {
        world.step();
        for beast in &mine {
            world.transform.get_mut(*beast).expect("stands").pos = out;
        }
    }
    assert!(beasts_of(&world, CAMP) > first, "an empty box stacks");
}

#[test]
fn a_neutral_sleeps_until_something_comes_right_up_to_it() {
    let camp = Vec2::from_ints(5000, 5000);
    for (apart, wakes) in [
        (rules::NEUTRAL_AGGRO_RANGE + 120, false),
        (rules::NEUTRAL_AGGRO_RANGE - 60, true),
    ] {
        let mut world = World::new();
        world.tick = rules::PREGAME_TICKS;
        let beasts = a_camp_at(&mut world, camp);
        let hero = hero_at(&mut world, camp + Vec2::from_ints(apart, 0));
        world.settle();
        world.step();
        world.step();
        assert_eq!(
            world.target_of(beasts[0]) == Some(hero),
            wakes,
            "standing {apart} off by day, waking should be {wakes}"
        );
    }
}

#[test]
fn a_neutral_sleeps_through_the_night_with_an_enemy_at_its_side_and_wakes_at_dawn() {
    let camp = Vec2::from_ints(5000, 5000);
    let dawn = rules::PREGAME_TICKS + 2 * rules::HALF_DAY_TICKS;
    let mut world = World::new();
    world.tick = dawn - 40;
    let beasts = a_camp_at(&mut world, camp);
    // An onlooker that never swings: an idle hero swings at what stands in
    // its reach, and the blow would wake the camp by itself.
    const ONLOOKER: UnitDef = UnitDef {
        acquisition: 0,
        ..MELEE_CREEP
    };
    let onlooker = world.spawn_unit(
        &ONLOOKER,
        Team::Radiant,
        camp + Vec2::from_ints(rules::NEUTRAL_AGGRO_RANGE - 60, 0),
    );
    world.settle();
    while world.tick < dawn - 1 {
        world.step();
        assert!(
            beasts.iter().all(|beast| world.target_of(*beast).is_none()),
            "asleep at night on tick {}, whoever stands beside it",
            world.tick
        );
    }
    world.step();
    world.step();
    assert_eq!(
        world.target_of(beasts[0]),
        Some(onlooker),
        "the day wakes it to what stands beside it"
    );
}

#[test]
fn a_blow_wakes_a_camp_from_further_than_it_can_see() {
    let mut world = World::new();
    let camp = Vec2::from_ints(5000, 5000);
    let beasts = a_camp_at(&mut world, camp);
    // Far past anything they could see, but inside the reach of a blow.
    let apart = rules::NEUTRAL_DAMAGE_AGGRO_RANGE - 100;
    let hero = hero_at(&mut world, camp + Vec2::from_ints(apart, 0));
    world.settle();
    world.step();
    assert!(
        !world.can_see(Team::Neutral, hero),
        "they cannot see that far, which is the point"
    );
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "and they sleep through it"
    );
    world.push_hit(Some(hero), beasts[0], 10, DamageKind::Physical);
    world.step();
    world.step();
    for beast in &beasts {
        assert_eq!(
            world.target_of(*beast),
            Some(hero),
            "a blow wakes the whole camp, seen or not"
        );
    }
    // Further off than a blow carries, it wakes nobody.
    let mut world = World::new();
    let beasts = a_camp_at(&mut world, camp);
    let far = hero_at(
        &mut world,
        camp + Vec2::from_ints(rules::NEUTRAL_DAMAGE_AGGRO_RANGE + 400, 0),
    );
    world.settle();
    world.push_hit(Some(far), beasts[0], 10, DamageKind::Physical);
    world.step();
    world.step();
    assert!(
        beasts.iter().all(|beast| world.target_of(*beast).is_none()),
        "a blow from beyond its reach wakes nobody"
    );
}

#[test]
fn a_neutral_slots_only_its_own_abilities_and_grows_with_the_clock_unseen() {
    // Dota's neutral_upgrade is an attribute-kind ability its HUD never
    // shows, and its modifier is hidden: a neutral's slots hold its own
    // abilities, and the clock's upgrades show only in its stats.
    let mut world = World::new();
    let horn = rules::tick_at_game_minute(0);
    world.tick = horn + rules::NEUTRAL_UPGRADE_PERIOD_TICKS - 2;
    let troll = world.spawn_neutral(
        NeutralKind::ForestTrollBerserker,
        Vec2::from_ints(5000, 5000),
        0,
        0,
    );
    world.settle();
    let before = shown(&world, None, troll);
    assert_eq!(
        before
            .abilities
            .iter()
            .map(|slot| slot.id)
            .collect::<Vec<_>>(),
        [NeutralAbility::Break.id(), NeutralAbility::Piercing.id()],
        "its slots are its own"
    );
    run(&mut world, 2);
    let after = shown(&world, None, troll);
    assert_eq!(
        (after.max_hp, after.attack_damage_min),
        (
            before.max_hp + rules::NEUTRAL_UPGRADE_HP,
            before.attack_damage_min + rules::NEUTRAL_UPGRADE_DAMAGE
        ),
        "the clock's upgrade raised it"
    );
    assert!(
        after
            .effects
            .iter()
            .all(|effect| effect.id != EffectId::NEUTRAL_UPGRADE),
        "and no view names the upgrade"
    );
}
