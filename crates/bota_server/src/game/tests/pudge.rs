//! Pudge.

use bota_core::{ModifierKind, rules, tower_def};

use crate::game::{Entity, Modifier, StackKind, Stacks, World, wire_id};
use bota_proto::{AbilityId, Fixed, HeroId, SlotId, Target, Team, Vec2};

use super::support::*;

#[test]
fn a_hook_catches_what_it_flies_into_and_drags_it_home() {
    let (mut world, pudge, mark) = pudge_and_a_mark(600);
    let full = world.health.get(mark).expect("standing").hp;
    let home = world.transform.get(pudge).expect("standing").pos;
    cast(&mut world, 0, Target::Pos(Vec2::from_ints(6000, 5000)));
    for _ in 0..120 {
        world.step();
        if !hook_flying(&world) {
            break;
        }
    }
    let landed = world.transform.get(mark).expect("standing").pos;
    assert!(
        landed.within(home, Fixed::from_int(200)),
        "what it caught is dragged to the one who threw it: {landed:?} against {home:?}"
    );
    assert!(
        world.health.get(mark).expect("standing").hp < full,
        "and an enemy feels it"
    );
}

#[test]
fn a_hook_that_catches_nothing_comes_back_by_itself_with_its_chain() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(600);
    // Thrown the other way, so it flies out over open ground and returns.
    cast(&mut world, 0, Target::Pos(Vec2::from_ints(4000, 5000)));
    world.step();
    let chained = |world: &World| {
        world
            .view_full()
            .projectiles
            .iter()
            .filter(|shown| shown.ability == Some(AbilityId::MEAT_HOOK))
            .count()
    };
    assert_eq!(
        chained(&world),
        rules::HOOK_LINKS + 1,
        "the hook and every link of its chain are in the view"
    );
    let mut caught = false;
    for _ in 0..200 {
        world.step();
        if !hook_flying(&world) {
            break;
        }
        caught |= world.entities.iter().any(|entity| {
            world
                .hook
                .get(entity)
                .is_some_and(|hook| hook.caught.is_some())
        });
    }
    assert!(!caught, "it caught nobody over open ground");
    assert!(!hook_flying(&world), "and came back");
    assert_eq!(chained(&world), 0, "the chain went home with the hook");
    assert!(world.alive(pudge));
}

#[test]
fn a_hook_flies_no_further_than_it_reaches() {
    let (mut world, _pudge, _mark) = pudge_and_a_mark(4000);
    // At the very edge of its reach: further off, the caster walks in first.
    cast(
        &mut world,
        0,
        Target::Pos(Vec2::from_ints(5000 + rules::HOOK_RANGE, 5000)),
    );
    let mut furthest = 0;
    for _ in 0..200 {
        world.step();
        for entity in world.entities.iter() {
            if world.hook.get(entity).is_some()
                && let Some(at) = world.transform.get(entity)
            {
                furthest = furthest.max(at.pos.x.to_int() - 5000);
            }
        }
    }
    assert!(
        furthest <= rules::HOOK_RANGE,
        "it stops at its own reach: {furthest} of {}",
        rules::HOOK_RANGE
    );
    assert!(
        furthest > rules::HOOK_RANGE - 100,
        "and gets there: {furthest}"
    );
}

#[test]
fn the_rot_burns_and_slows_what_stands_in_it_and_lifts_when_switched_off() {
    let (mut world, pudge, mark) = pudge_and_a_mark(150);
    set_level(&mut world, pudge, 1, 1);
    let full = world.health.get(mark).expect("standing").hp;
    cast(&mut world, 1, Target::None);
    assert!(
        carries(&world, pudge, ModifierKind::Rot { level: 0 }),
        "it is switched on"
    );
    run(&mut world, rules::BURN_PERIOD_TICKS * 4);
    assert!(
        world.health.get(mark).expect("standing").hp < full,
        "what stands in it burns"
    );
    assert!(
        carries(&world, mark, ModifierKind::Slowed { pct: 0 }),
        "and is slowed while it stands there"
    );
    cast(&mut world, 1, Target::None);
    assert!(
        !carries(&world, pudge, ModifierKind::Rot { level: 0 }),
        "it is switched off"
    );
    run(&mut world, 3);
    assert!(
        !carries(&world, mark, ModifierKind::Slowed { pct: 0 }),
        "and nothing is left slowed"
    );
}

#[test]
fn the_rot_shows_its_cloud_where_its_owner_stands_while_it_is_on() {
    // The mark stands too far off to be taken on, so he only moves when
    // moved.
    let (mut world, pudge, _mark) = pudge_and_a_mark(2000);
    set_level(&mut world, pudge, 1, 1);
    cast(&mut world, 1, Target::None);
    let cloud = world
        .mark_of(pudge, AbilityId::ROT)
        .expect("switched on, the cloud shows");
    let stood = world.transform.get(pudge).expect("standing").pos;
    assert_eq!(world.transform.get(cloud).map(|t| t.pos), Some(stood));
    world.transform.get_mut(pudge).expect("standing").pos = stood + Vec2::from_ints(0, 90);
    world.step();
    assert_eq!(
        world.transform.get(cloud).map(|t| t.pos),
        world.transform.get(pudge).map(|t| t.pos),
        "and follows him"
    );
    assert!(
        world
            .view_full()
            .projectiles
            .iter()
            .any(|shown| shown.ability == Some(AbilityId::ROT)),
        "and is in the view"
    );
    cast(&mut world, 1, Target::None);
    assert_eq!(
        world.mark_of(pudge, AbilityId::ROT),
        None,
        "switched off, it goes"
    );
    cast(&mut world, 1, Target::None);
    assert!(world.mark_of(pudge, AbilityId::ROT).is_some());
    world.bury(vec![(pudge, None)], &mut Vec::new());
    assert!(
        world.entities.iter().all(|entity| world
            .mark
            .get(entity)
            .is_none_or(|shown| shown.ability != AbilityId::ROT)),
        "and it goes with him when he falls"
    );
}

#[test]
fn the_rot_never_kills_the_one_carrying_it() {
    let (mut world, pudge) = a_lone_pudge();
    set_level(&mut world, pudge, 1, 3);
    world.put_modifier(
        pudge,
        Modifier {
            kind: ModifierKind::Rot { level: 3 },
            source: Some(pudge),
            ticks_left: None,
        },
    );
    world.health.insert(pudge, health(20));
    run(&mut world, rules::BURN_PERIOD_TICKS * 40);
    assert!(
        world.health.get(pudge).expect("standing").hp.to_int() < 20,
        "it does burn its owner"
    );
    assert!(world.alive(pudge), "but never kills him");
    assert!(
        world.health.get(pudge).expect("standing").hp >= Fixed::from_int(1),
        "and never takes his last point"
    );
}

#[test]
fn a_dismember_holds_what_it_eats_and_feeds_the_one_eating() {
    let (mut world, pudge, mark) = pudge_and_a_mark(100);
    set_level(&mut world, pudge, 3, 1);
    world.health.insert(pudge, health(300));
    world.step();
    let hurt = world.health.get(pudge).expect("standing").hp;
    let full = world.health.get(mark).expect("standing").hp;
    cast(&mut world, 3, Target::Unit(wire_id(mark)));
    assert!(world.is_channelling(pudge), "it takes hold");
    run(&mut world, 30);
    assert!(
        carries(&world, mark, ModifierKind::Stunned),
        "what it holds cannot act"
    );
    assert!(
        world.health.get(mark).expect("standing").hp < full,
        "and is eaten"
    );
    assert!(
        world.health.get(pudge).expect("standing").hp > hurt,
        "while the one eating mends"
    );
    stop_click(&mut world);
    assert!(!world.is_channelling(pudge), "an order of any kind lets go");
}

#[test]
fn a_dismember_shows_its_hold_on_what_it_eats_until_it_lets_go() {
    let (mut world, pudge, mark) = pudge_and_a_mark(100);
    set_level(&mut world, pudge, 3, 1);
    cast(&mut world, 3, Target::Unit(wire_id(mark)));
    let shown = world
        .mark_of(pudge, AbilityId::DISMEMBER)
        .expect("the hold is shown");
    let eaten = world.transform.get(mark).expect("standing").pos;
    assert_eq!(world.transform.get(shown).map(|t| t.pos), Some(eaten));
    world.transform.get_mut(mark).expect("standing").pos = eaten + Vec2::from_ints(40, 0);
    world.step();
    assert_eq!(
        world.transform.get(shown).map(|t| t.pos),
        world.transform.get(mark).map(|t| t.pos),
        "and follows what is eaten"
    );
    stop_click(&mut world);
    assert_eq!(
        world.mark_of(pudge, AbilityId::DISMEMBER),
        None,
        "letting go takes the hold away"
    );
}

#[test]
fn a_flesh_heap_keeps_every_enemy_hero_that_falls_near_it_and_nothing_else() {
    let (mut world, pudge, mark) = pudge_and_a_mark(200);
    set_level(&mut world, pudge, 2, 1);
    let foe = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5300, 5000),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    let friend = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5300),
        SlotId(2),
        HeroId::SHADOW_FIEND,
    );
    let far = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5000 + rules::FLESH_HEAP_RANGE + 200, 5000),
        SlotId(3),
        HeroId::SHADOW_FIEND,
    );
    let tower = world.spawn_unit(tower_def(1), Team::Dire, Vec2::from_ints(5200, 5000));
    world.settle();
    world.step();
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    let mut events = Vec::new();
    world.bury(vec![(mark, None), (tower, None)], &mut events);
    world.step();
    assert_eq!(
        heap(&world, pudge),
        0,
        "a creep or a structure falling beside it feeds it nothing"
    );
    world.bury(vec![(friend, None), (far, None)], &mut events);
    world.step();
    assert_eq!(
        heap(&world, pudge),
        0,
        "neither a hero of its own side, nor an enemy hero falling too far off"
    );
    // Brought down the way a fight brings a hero down: dead before it is
    // buried.
    world.health.insert(foe, health(0));
    world.bury(vec![(foe, None)], &mut events);
    world.step();
    assert_eq!(
        heap(&world, pudge),
        1,
        "an enemy hero falling beside it feeds it"
    );
    assert_eq!(
        world.stats.get(pudge).map(|s| s.attributes.strength),
        Some(bare + rules::FLESH_HEAP_STRENGTH_PER_STACK[0]),
        "and one stack is worth its strength at the heap's first level"
    );
}

/// More strength a stack at each level, and magic resistance that multiplies
/// with the hero's own rather than adding to it.
#[test]
fn a_thicker_heap_is_worth_more_a_stack_and_turns_more_magic() {
    let (mut world, pudge) = a_lone_pudge();
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    assert_eq!(
        world.stats.get(pudge).map(|s| s.magic_resist_bp),
        Some(rules::HERO_MAGIC_RESIST_PCT * 100),
        "unlearned, the heap thickens nothing"
    );
    set_level(&mut world, pudge, 2, 4);
    let mut kept = Stacks::default();
    kept.gather(StackKind::FleshHeap, 3);
    world.stacks.insert(pudge, kept);
    world.step();
    assert_eq!(
        world.stats.get(pudge).map(|s| s.attributes.strength),
        Some(bare + Fixed::from_int(9)),
        "three stacks at the fourth level are worth three strength each"
    );
    let base = rules::HERO_MAGIC_RESIST_PCT;
    let heap = rules::FLESH_HEAP_MAGIC_RESIST_PCT[3];
    assert_eq!(
        world.stats.get(pudge).map(|s| s.magic_resist_bp),
        Some(rules::NOMINAL_BP - (rules::NOMINAL_BP - 100 * base) * (100 - heap) / 100),
        "and the resistance multiplies with the hero's own"
    );
}

#[test]
fn a_flesh_heap_outlives_the_death_of_the_one_carrying_it() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(200);
    set_level(&mut world, pudge, 2, 1);
    let foe = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5100, 5200),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    world.settle();
    world.step();
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    let mut events = Vec::new();
    world.bury(vec![(foe, None)], &mut events);
    world.step();
    assert_eq!(heap(&world, pudge), 1, "one death has fed it");
    world.bury(vec![(pudge, None)], &mut events);
    assert_eq!(world.seats[0].unit, None, "the body is gone");
    let back = back_on_its_feet(&mut world);
    assert_eq!(heap(&world, back), 1, "and what it kept comes back with it");
    assert_eq!(
        world.stats.get(back).map(|s| s.attributes.strength),
        Some(bare + rules::FLESH_HEAP_STRENGTH_PER_STACK[0]),
        "worth as much strength as it was before"
    );
}

/// Pudge alone on an empty world at (5000, 5000), settled and one tick on.
fn a_lone_pudge() -> (World, Entity) {
    let mut world = World::new();
    let pudge = world.spawn_hero(
        Team::Radiant,
        Vec2::from_ints(5000, 5000),
        SlotId(0),
        HeroId::PUDGE,
    );
    world.settle();
    world.step();
    (world, pudge)
}

fn set_level(world: &mut World, hero: Entity, slot: usize, level: u8) {
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[slot].level = level;
    }
}

fn heap(world: &World, hero: Entity) -> u32 {
    world
        .stacks
        .get(hero)
        .map_or(0, |kept| kept.of(StackKind::FleshHeap))
}
