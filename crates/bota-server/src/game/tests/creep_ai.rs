//! What creeps and towers choose to fight, and what pulls them.

use crate::game::rules;
use crate::game::{Health, MELEE_CREEP, World};
use bota_proto::{Fixed, Team};

use super::support::*;

#[test]
fn a_tower_takes_the_nearest_enemy_and_brings_it_down() {
    let mut world = World::new();
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    let near = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1200, 1000),
    );
    let far = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1600, 1000),
    );
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
    let at = bota_proto::Vec2::from_ints(1000, 1000);
    let creep = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Radiant, at);
    let siege = world.spawn_unit(&crate::game::SIEGE_CREEP, bota_proto::Team::Radiant, at);
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1100, 1000),
    );
    let enemy = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(1300, 1000),
    );
    world.settle();
    let reach = world.stats.get(creep).expect("settled").acquisition;
    assert_eq!(
        world.acquire(
            creep,
            reach,
            crate::game::PriorityOrder::Normal,
            &world.candidates()
        ),
        Some(enemy),
        "a unit outranks a building however much nearer the building stands"
    );
    let siege_reach = world.stats.get(siege).expect("settled").acquisition;
    assert_eq!(
        world.acquire(
            siege,
            siege_reach,
            crate::game::PriorityOrder::SiegeFirst,
            &world.candidates()
        ),
        Some(tower),
        "a siege creep goes for the building"
    );
    assert_eq!(
        world.priority_of(siege),
        crate::game::PriorityOrder::SiegeFirst
    );
    assert_eq!(world.priority_of(creep), crate::game::PriorityOrder::Normal);
}

#[test]
fn a_building_never_shoots_the_jungle_and_a_creep_only_at_a_pull_camp() {
    let mut world = World::new();
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    let creep = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
    );
    let hero = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(1000, 1000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let beast = world.spawn_unit(
        crate::game::NeutralKind::Kobold.def(),
        bota_proto::Team::Neutral,
        bota_proto::Vec2::from_ints(1100, 1000),
    );
    world.settle();
    assert!(!world.hostile(tower, beast), "a tower leaves the jungle be");
    assert!(world.hostile(hero, beast), "a hero may hit it");
    assert!(
        !world.hostile(creep, beast),
        "a lane creep leaves a camp it cannot be pulled to"
    );
    let pull = crate::game::CAMPS
        .iter()
        .find(|c| c.pullable)
        .expect("the map marks pull camps");
    world.camp_home.insert(
        beast,
        crate::game::CampHome {
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
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    let prey = world.spawn_unit(
        &MELEE_CREEP,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    world.settle();
    world.step();
    assert_eq!(world.target_of(creep), Some(prey), "it takes it");
    // Carried out of everything it can see, and the creep held where it was.
    let away = bota_proto::Vec2::from_ints(5900, 5000);
    for _ in 0..rules::CREEP_CHASE_TICKS + 2 {
        if let Some(at) = world.transform.get_mut(prey) {
            at.pos = away;
        }
        if let Some(at) = world.transform.get_mut(creep) {
            at.pos = bota_proto::Vec2::from_ints(5000, 5000);
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
    let at = bota_proto::Vec2::from_ints(5000, 5000);
    let creep = thinking_creep(&mut world, at);
    let hero = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5040, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    // Somebody else for the creep to fall back on, so that a hero put last
    // would show.
    world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
    );
    world.settle();
    // Swinging at the creep's own is what makes the creep look at a hero at
    // all; left alone it would take the creep and there would be nothing to
    // let go of.
    world.set_order(
        hero,
        crate::game::UnitOrder::Attack {
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
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    // The hero stands nearer than the enemy creep.
    let hero = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let other = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5000),
    );
    world.settle();
    assert_eq!(
        world.best_valid_in_range(creep, Fixed::from_int(600), &world.candidates()),
        Some(other),
        "what it is doing outranks how near it stands"
    );
    let _ = hero;
}

#[test]
fn a_hero_laying_into_your_side_counts_for_no_more_than_a_creep() {
    let mut world = World::new();
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    let friend = world.spawn_unit(
        &MELEE_CREEP,
        Team::Radiant,
        bota_proto::Vec2::from_ints(5050, 5000),
    );
    let hero = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    let other = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
    );
    world.settle();
    world.set_order(hero, crate::game::UnitOrder::Stand);
    assert_eq!(
        world.threat_priority(creep, hero),
        2,
        "doing nothing to this side, it comes after a plain unit"
    );
    // The hero lays into one of ours.
    world.set_order(
        hero,
        crate::game::UnitOrder::Attack {
            target: friend,
            last_seen: bota_proto::Vec2::from_ints(5050, 5000),
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
        at.pos = bota_proto::Vec2::from_ints(5040, 5000);
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
    let creep = thinking_creep(&mut world, bota_proto::Vec2::from_ints(5000, 5000));
    let theirs = world.spawn_unit(
        &MELEE_CREEP,
        Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5000),
    );
    let hero = world.spawn_hero(
        Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.orders.insert(
        hero,
        crate::game::Orders {
            current: crate::game::UnitOrder::Attack {
                target: theirs,
                last_seen: bota_proto::Vec2::from_ints(5400, 5000),
            },
            cooldown: 0,
            pending: None,
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
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(700);
    assert_eq!(world.target_of(theirs), Some(ours));
    attack_click(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "it never saw the order given"
    );
    let _ = hero;
}

#[test]
fn the_hold_lets_go_after_two_and_a_third_seconds() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    attack_click(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(hero));
    // The hero stops laying into that side.
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    for _ in 0..rules::ORDER_AGGRO_HOLD_TICKS - 4 {
        world.step();
        // Held where they were put, so only the clock decides.
        if let Some(at) = world.transform.get_mut(theirs) {
            at.pos = bota_proto::Vec2::from_ints(5300, 5000);
        }
    }
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "the hold has not run out yet"
    );
    for _ in 0..6 {
        world.step();
        if let Some(at) = world.transform.get_mut(theirs) {
            at.pos = bota_proto::Vec2::from_ints(5300, 5000);
        }
    }
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
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    for _ in 0..rules::ORDER_AGGRO_HOLD_TICKS + 2 {
        world.step();
        if let Some(at) = world.transform.get_mut(theirs) {
            at.pos = bota_proto::Vec2::from_ints(5300, 5000);
        }
    }
    assert_eq!(world.target_of(theirs), Some(ours), "it let go");
    assert!(
        world
            .orders
            .get(theirs)
            .is_some_and(|orders| orders.cooldown > 0),
        "and the wait before it answers again still runs"
    );
    attack_click(&mut world, theirs);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "so a second click inside that wait pulls nothing"
    );
}

#[test]
fn clicking_your_own_does_not_pull_the_creeps_onto_you() {
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    let _ = foe;
    // Worn down far enough to be worth denying.
    let max = world.stats.get(ours).expect("settled").max_hp;
    world.health.insert(
        ours,
        Health {
            hp: Fixed {
                raw: max.raw * 40 / 100,
            },
        },
    );
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
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    for _ in 0..rules::ORDER_AGGRO_HOLD_TICKS + 2 {
        world.step();
        if let Some(at) = world.transform.get_mut(theirs) {
            at.pos = bota_proto::Vec2::from_ints(5300, 5000);
        }
    }
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "and lets go in its time"
    );
}

#[test]
fn a_spell_answers_the_way_a_swing_does_and_never_lets_go() {
    // At an enemy hero it calls them on.
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "left alone, creep on creep"
    );
    cast_at(&mut world, foe);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "a spell at an enemy hero calls them on"
    );
    // At an enemy creep it moves nobody.
    let (mut world, _hero, theirs, ours, _foe) = a_lane_with_a_hero(300);
    cast_at(&mut world, theirs);
    assert_eq!(
        world.target_of(theirs),
        Some(ours),
        "a spell at an enemy creep is a spell like any other"
    );
    // At one of your own it is not their business: what holds them holds.
    let (mut world, hero, theirs, ours, foe) = a_lane_with_a_hero(300);
    cast_at(&mut world, foe);
    assert_eq!(world.target_of(theirs), Some(hero), "called on");
    cast_at(&mut world, ours);
    assert_eq!(
        world.target_of(theirs),
        Some(hero),
        "and a spell at one of your own does not let them go"
    );
}

#[test]
fn the_jungle_pays_a_courier_no_mind_but_the_other_side_does_not() {
    let mut world = World::for_match(&config(), config().rng());
    let courier = the_courier(&world);
    let at = world.transform.get(courier).expect("standing").pos;
    // A neutral and a creep of the other side, both standing on top of it.
    let beast = world.spawn_unit(
        crate::game::NeutralKind::Kobold.def(),
        bota_proto::Team::Neutral,
        at,
    );
    world
        .camp_home
        .insert(beast, crate::game::CampHome { camp: 0, home: at });
    world.neutral_ai.insert(
        beast,
        crate::game::NeutralAi {
            leash_left: rules::NEUTRAL_AGGRO_WINDOW,
            reaggro_block: 0,
            next_window: rules::NEUTRAL_AGGRO_WINDOW,
            going_home: false,
            roused_by: None,
            awake: true,
        },
    );
    let creep = world.spawn_unit(&MELEE_CREEP, bota_proto::Team::Dire, at);
    world.settle();
    for _ in 0..10 {
        world.step();
    }
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
