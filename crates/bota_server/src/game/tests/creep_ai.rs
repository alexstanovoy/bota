//! What creeps and towers choose to fight, and what pulls them.

use bota_core::{CAMPS, MELEE_CREEP, NeutralKind, PriorityOrder, SIEGE_CREEP, rules, tower_def};

use crate::game::{CampHome, Entity, NeutralAi, UnitOrder, World, wire_id};
use bota_proto::{Fixed, HeroId, SlotId, Target, Team, Vec2};

use super::support::*;

/// Steps the world, putting a creep back where the lane fixture stood it
/// after every tick, so only the clock decides what it does.
fn held_in_place(world: &mut World, creep: Entity, ticks: u32) {
    for _ in 0..ticks {
        world.step();
        if let Some(at) = world.transform.get_mut(creep) {
            at.pos = Vec2::from_ints(5300, 5000);
        }
    }
}

#[test]
fn a_tower_takes_the_nearest_enemy_and_brings_it_down() {
    let mut world = World::new();
    let tower = world.spawn_unit(tower_def(1), Team::Radiant, Vec2::from_ints(1000, 1000));
    let near = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(1200, 1000));
    let far = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(1600, 1000));
    world.settle();
    world.step();
    assert_eq!(
        world.target_of(tower),
        Some(near),
        "the nearer one is taken"
    );
    let mut ticks = 0;
    while world.alive(near) && ticks < 900 {
        world.step();
        ticks += 1;
    }
    assert!(!world.alive(near), "a tower brings a creep down");
    assert!(world.alive(far), "the other one was never in reach");
    assert!(!world.entities.contains(near), "what falls is cleared away");
}

#[test]
fn a_unit_is_taken_before_a_building_and_a_siege_creep_takes_the_building_first() {
    let mut world = World::new();
    let at = Vec2::from_ints(1000, 1000);
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, at);
    let siege = world.spawn_unit(&SIEGE_CREEP, Team::Radiant, at);
    let tower = world.spawn_unit(tower_def(1), Team::Dire, Vec2::from_ints(1100, 1000));
    let enemy = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(1300, 1000));
    world.settle();
    let reach = world.stats.get(creep).expect("settled").acquisition;
    assert_eq!(
        world.acquire(creep, reach, PriorityOrder::Normal, &world.candidates()),
        Some(enemy),
        "a unit outranks a building however much nearer the building stands"
    );
    let siege_reach = world.stats.get(siege).expect("settled").acquisition;
    assert_eq!(
        world.acquire(
            siege,
            siege_reach,
            PriorityOrder::SiegeFirst,
            &world.candidates()
        ),
        Some(tower),
        "a siege creep goes for the building"
    );
    assert_eq!(world.priority_of(siege), PriorityOrder::SiegeFirst);
    assert_eq!(world.priority_of(creep), PriorityOrder::Normal);
}

#[test]
fn a_building_never_shoots_the_jungle_and_a_creep_only_at_a_pull_camp() {
    let mut world = World::new();
    let tower = world.spawn_unit(tower_def(1), Team::Radiant, Vec2::from_ints(1000, 1000));
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(1000, 1000));
    let hero = hero_at(&mut world, Vec2::from_ints(1000, 1000));
    let beast = world.spawn_unit(
        NeutralKind::Kobold.def(),
        Team::Neutral,
        Vec2::from_ints(1100, 1000),
    );
    world.settle();
    assert!(!world.hostile(tower, beast), "a tower leaves the jungle be");
    assert!(world.hostile(hero, beast), "a hero may hit it");
    assert!(
        !world.hostile(creep, beast),
        "a lane creep leaves a camp it cannot be pulled to"
    );
    let pull = CAMPS
        .iter()
        .find(|c| c.pullable)
        .expect("the map marks pull camps");
    world.camp_home.insert(
        beast,
        CampHome {
            camp: 0,
            home: pull.pos,
        },
    );
    assert!(
        world.hostile(creep, beast),
        "at a pull camp it will fight after all"
    );
}

#[test]
fn a_creep_gives_up_a_chase_it_cannot_finish() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, Vec2::from_ints(5000, 5000));
    let prey = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.step();
    assert_eq!(world.target_of(creep), Some(prey), "it takes it");
    // Carried out of everything it can see, and the creep held where it was.
    let away = Vec2::from_ints(5900, 5000);
    for _ in 0..rules::CREEP_CHASE_TICKS + 2 {
        if let Some(at) = world.transform.get_mut(prey) {
            at.pos = away;
        }
        if let Some(at) = world.transform.get_mut(creep) {
            at.pos = Vec2::from_ints(5000, 5000);
        }
        world.step();
    }
    assert_eq!(
        world.target_of(creep),
        None,
        "the chase ran out and it let go"
    );
}

#[test]
fn an_attack_order_a_hero_aims_at_itself_moves_nobody() {
    let mut world = World::new();
    let at = Vec2::from_ints(5000, 5000);
    let creep = thinking_creep(&mut world, at);
    let hero = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5040, 5000),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    // Somebody else for the creep to fall back on, so that a hero put last
    // would show.
    world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5300, 5000));
    world.settle();
    // Swinging at the creep's own is what makes the creep look at a hero at
    // all; left alone it would take the creep and there would be nothing to
    // let go of.
    world.set_order(
        hero,
        UnitOrder::Attack {
            target: creep,
            last_seen: at,
        },
    );
    world.step();
    assert_eq!(
        world.target_of(creep),
        Some(hero),
        "the creep answers the hero swinging at its own"
    );
    world.rouse_bystanders(hero, hero);
    world.step();
    assert_eq!(
        world.target_of(creep),
        Some(hero),
        "pointing at itself is not the way a hero lets creeps go"
    );
}

#[test]
fn a_creep_prefers_a_creep_to_a_hero_that_is_doing_nothing() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, Vec2::from_ints(5000, 5000));
    // The hero stands nearer than the enemy creep.
    world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5100, 5000),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    let other = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5400, 5000));
    world.settle();
    assert_eq!(
        world.best_valid_in_range(creep, Fixed::from_int(600), &world.candidates()),
        Some(other),
        "what it is doing outranks how near it stands"
    );
}

#[test]
fn a_hero_laying_into_your_side_counts_for_no_more_than_a_creep() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, Vec2::from_ints(5000, 5000));
    let friend = world.spawn_unit(&MELEE_CREEP, Team::Radiant, Vec2::from_ints(5050, 5000));
    let hero = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5300, 5000),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    let other = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5100, 5000));
    world.settle();
    world.set_order(hero, UnitOrder::Stand);
    assert_eq!(
        world.threat_priority(creep, hero),
        2,
        "doing nothing to this side, it comes after a plain unit"
    );
    // The hero lays into one of ours.
    world.set_order(
        hero,
        UnitOrder::Attack {
            target: friend,
            last_seen: Vec2::from_ints(5050, 5000),
        },
    );
    assert_eq!(
        world.threat_priority(creep, hero),
        world.threat_priority(creep, other),
        "laying into this side, it counts the same as a creep"
    );
    assert_eq!(
        world.best_valid_in_range(creep, Fixed::from_int(600), &world.candidates()),
        Some(other),
        "so the nearer of the two wins, and that is the creep"
    );
    // With the hero the nearer of the two, it is the one taken.
    if let Some(at) = world.transform.get_mut(hero) {
        at.pos = Vec2::from_ints(5040, 5000);
    }
    assert_eq!(
        world.best_valid_in_range(creep, Fixed::from_int(600), &world.candidates()),
        Some(hero),
        "nearness decides between equals"
    );
}

#[test]
fn a_hero_putting_out_its_own_is_taken_on_last() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, Vec2::from_ints(5000, 5000));
    let theirs = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(5400, 5000));
    let hero = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5100, 5000),
        SlotId(0),
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    world.set_order(
        hero,
        UnitOrder::Attack {
            target: theirs,
            last_seen: Vec2::from_ints(5400, 5000),
        },
    );
    assert_eq!(
        world.threat_priority(creep, hero),
        3,
        "putting out its own puts it last"
    );
    assert_eq!(
        world.best_valid_in_range(creep, Fixed::from_int(600), &world.candidates()),
        Some(theirs),
        "so the creep it was denying is taken on instead"
    );
}

#[test]
fn attacking_an_enemy_pulls_the_creeps_near_you_onto_you() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "left alone, the creep fights the creep"
    );
    attack_click(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "the click pulls it onto whoever gave it"
    );
}

#[test]
fn attacking_an_enemy_creep_pulls_nobody() {
    let (mut world, _hero, theirs, ours, _foe) = a_lane_with_a_hero(300);
    // Clicking the enemy creep is a last hit, and a last hit is not a call.
    attack_click(&mut world, theirs);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "the creep goes on fighting what it was fighting"
    );
}

#[test]
fn a_creep_too_far_off_pays_the_order_no_mind() {
    // Past a melee creep's acquisition of 500.
    let (mut world, _hero, theirs, ours, foe) = a_lane_with_a_hero(700);
    assert_eq!(world.target_of(theirs), Some(ours));
    attack_click(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "it never saw the order given"
    );
}

#[test]
fn the_hold_lets_go_after_two_point_three_seconds() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    attack_click(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(hero));
    // The hero stops laying into that side.
    stop_click(&mut world);
    held_in_place(&mut world, theirs, rules::ORDER_AGGRO_HOLD_TICKS - 4);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "the hold has not run out yet"
    );
    held_in_place(&mut world, theirs, 6);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "past it, the ranking takes the creep back"
    );
}

#[test]
fn a_second_click_inside_the_wait_pulls_nothing() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    attack_click(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(hero));
    // Stop, wait out the hold, and click again while the wait still runs.
    stop_click(&mut world);
    held_in_place(&mut world, theirs, rules::ORDER_AGGRO_HOLD_TICKS + 2);
    assert_eq!(world.target_of(theirs), Some(ours), "it let go");
    attack_click(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "so a second click inside that wait pulls nothing"
    );
}

#[test]
fn the_wait_is_the_heros_and_starts_with_no_creep_near() {
    // Liquipedia: the 3 s cooldown always triggers, even with no creeps
    // nearby, and each hero keeps its own.
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(700);
    attack_click(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(ours), "too far to hear it");
    stop_click(&mut world);
    let near = Vec2::from_ints(5000 + 700 - 300, 5000);
    let walk_in = |world: &mut World| {
        if let Some(at) = world.transform.get_mut(hero) {
            at.pos = near;
        }
    };
    walk_in(&mut world);
    attack_click(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "near now, but the far click started the hero's wait"
    );
    stop_click(&mut world);
    for _ in 0..rules::ORDER_AGGRO_COOLDOWN_TICKS {
        world.step();
        walk_in(&mut world);
    }
    attack_click(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "once it has run out, the next click pulls"
    );
}

#[test]
fn clicking_your_own_does_not_pull_the_creeps_onto_you() {
    let (mut world, hero, theirs, ours, _foe) = a_lane_with_a_hero(300);
    // Worn down far enough to be worth denying.
    wear_down(&mut world, ours, 40);
    attack_click(&mut world, ours);
    assert_eq!(
        world.target_of(hero),
        Some(ours),
        "the hero does go for the deny"
    );
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "but the creep is not pulled onto the one who clicked"
    );
}

#[test]
fn a_hold_is_not_broken_by_clicking_your_own() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    attack_click(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(hero), "pulled onto the hero");
    // Straight away, click one of your own: the hold does not give.
    attack_click(&mut world, ours);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "what pulled it keeps it for the whole span"
    );
    assert_eq!(
        world.target_of(hero),
        None,
        "and the hero strikes nothing of its own"
    );
    // Once the hold is out, the ranking takes the creep back on its own.
    stop_click(&mut world);
    held_in_place(&mut world, theirs, rules::ORDER_AGGRO_HOLD_TICKS + 2);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "and lets go in its time"
    );
}

#[test]
fn a_spell_at_a_hero_draws_nobody() {
    // Liquipedia, Lane Creeps: the creeps' protective behaviour reacts on
    // attacks and completely ignores ability casts.
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "left alone, creep on creep"
    );
    cast(&mut world, 3, Target::Unit(wire_id(foe)));
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "a spell at an enemy hero calls nobody on"
    );
    attack_click(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "and leaves the hero's order wait unspent"
    );
}

#[test]
fn the_jungle_pays_a_courier_no_mind_but_the_other_side_does_not() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    // Out of its fountain, whose aura would make it invulnerable.
    let at = rules::DEMO_LANE_CORNERS[0];
    world.transform.get_mut(courier).expect("standing").pos = at;
    // A neutral and a creep of the other side, both standing on top of it.
    let beast = world.spawn_unit(NeutralKind::Kobold.def(), Team::Neutral, at);
    world
        .camp_home
        .insert(beast, CampHome { camp: 1, home: at });
    world.neutral_ai.insert(
        beast,
        NeutralAi {
            leash_left: rules::NEUTRAL_AGGRO_WINDOW,
            reaggro_block: 0,
            next_window: rules::NEUTRAL_AGGRO_WINDOW,
            going_home: false,
            roused_by: None,
            awake: true,
        },
    );
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, at);
    world.settle();
    run(&mut world, 10);
    assert!(
        !world.hostile(beast, courier),
        "the jungle does not take a courier on"
    );
    assert_ne!(world.target_of(beast), Some(courier));
    assert!(
        world.hostile(creep, courier),
        "a creep of the other side does"
    );
    assert_eq!(world.target_of(creep), Some(courier), "and goes for it");
}

/// A cliff on the plain map: a spot on low ground, one on higher ground
/// 256 to 384 straight off it, and two more spots on the same low ground
/// 64 and 192 further from the cliff.
fn a_cliff(world: &World) -> (Vec2, Vec2, Vec2, Vec2) {
    let ground = &world.ground;
    let centre = |x: usize, y: usize| {
        Vec2::from_ints(
            x as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
            y as i32 * rules::GRID_CELL_SIZE + rules::GRID_CELL_SIZE / 2,
        )
    };
    let cells = crate::game::TERRAIN_CELLS as i64;
    for y in 8..cells - 8 {
        for x in 8..cells - 8 {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                for far in 4..=6 {
                    let at = |k: i64| ((x + dx * k) as usize, (y + dy * k) as usize);
                    let [low, high, near, back] = [at(0), at(far), at(-1), at(-3)];
                    let walkable = [low, high, near, back]
                        .iter()
                        .all(|&(cx, cy)| ground.cell_walkable(cx, cy));
                    let tier = |(cx, cy)| ground.tier(centre(cx, cy));
                    if walkable
                        && tier(high) > tier(low)
                        && tier(near) == tier(low)
                        && tier(back) == tier(low)
                    {
                        let spot = |(cx, cy)| centre(cx, cy);
                        return (spot(low), spot(high), spot(near), spot(back));
                    }
                }
            }
        }
    }
    panic!("the plain map has a cliff");
}

/// A seated Radiant archer on top of a cliff and, below it, a Dire lane
/// creep fighting a Radiant one with a Dire hero beside them; the clock past
/// the early-aggro rule and one tick run.
fn a_lane_under_a_cliff() -> (World, Entity, Entity, Entity, Entity) {
    let mut world = World::new();
    world.tick = rules::FREE_AGGRO_TICK;
    let (low, high, near, back) = a_cliff(&world);
    let hero = seated_hero(&mut world, high, HeroId::SHADOW_FIEND);
    world.set_order(hero, UnitOrder::Stand);
    let theirs = thinking_creep_of(&mut world, Team::Dire, low);
    let ours = world.spawn_unit(&MELEE_CREEP, Team::Radiant, near);
    let foe = world.spawn_hero(Team::Dire, back, SlotId(1), HeroId::SHADOW_FIEND);
    world.set_order(foe, UnitOrder::Stand);
    world.settle();
    world.step();
    assert!(
        !world.can_see(Team::Dire, hero),
        "the archer stands in the fog of the low ground"
    );
    assert_eq!(world.target_of(theirs), Some(ours), "the creeps fight");
    (world, hero, theirs, ours, foe)
}

#[test]
fn a_click_from_high_ground_pulls_the_creeps_into_the_fog_after_it() {
    let (mut world, hero, theirs, _ours, foe) = a_lane_under_a_cliff();
    attack_click(&mut world, foe);
    for _ in 0..rules::ORDER_AGGRO_HOLD_TICKS - 1 {
        assert_eq!(
            world.target_of(theirs),
            Some(hero),
            "tick {}: the hold chases the archer, seen or not",
            world.tick
        );
        world.step();
    }
    assert!(
        world.exposure.get(hero).is_some(),
        "the archer has struck by now"
    );
}

#[test]
fn every_swing_from_high_ground_pulls_the_creeps_that_see_it() {
    let (mut world, hero, theirs, ours, foe) = a_lane_under_a_cliff();
    // No click: an order set in place calls nobody on, so only the swings do.
    let last_seen = world.transform.get(foe).expect("standing").pos;
    world.set_order(
        hero,
        UnitOrder::Attack {
            target: foe,
            last_seen,
        },
    );
    let mut pulled = false;
    for _ in 0..4 * rules::TICKS_PER_SECOND {
        let shown = world.can_see(Team::Dire, hero);
        world.step();
        let onto = world.target_of(theirs);
        if !shown {
            assert_eq!(onto, Some(ours), "a swing from the fog pulls nobody");
        }
        pulled |= onto == Some(hero);
    }
    assert!(
        pulled,
        "once its first strike shows it, the next swing pulls the creep"
    );
}
