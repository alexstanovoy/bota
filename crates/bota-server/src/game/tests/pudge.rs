//! Pudge.

use crate::game::rules;
use crate::game::{Health, StackKind, World};
use bota_proto::Fixed;

use super::support::*;

#[test]
fn a_hook_catches_what_it_flies_into_and_drags_it_home() {
    let (mut world, pudge, mark) = pudge_and_a_mark(600);
    let full = world.health.get(mark).expect("standing").hp.to_int();
    let home = world.transform.get(pudge).expect("standing").pos;
    throw_hook(&mut world, bota_proto::Vec2::from_ints(6000, 5000));
    for _ in 0..120 {
        world.step();
        if world.entities.iter().all(|e| world.hook.get(e).is_none()) {
            break;
        }
    }
    let landed = world.transform.get(mark).expect("standing").pos;
    assert!(
        landed.within(home, bota_proto::Fixed::from_int(200)),
        "what it caught is dragged to the one who threw it: {landed:?} against {home:?}"
    );
    assert!(
        world.health.get(mark).expect("standing").hp.to_int() < full,
        "and an enemy feels it"
    );
}

#[test]
fn a_hook_that_catches_nothing_comes_back_by_itself() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(600);
    // Thrown the other way, so it flies out over open ground and returns.
    throw_hook(&mut world, bota_proto::Vec2::from_ints(4000, 5000));
    let (mut flew, mut caught) = (false, false);
    for _ in 0..200 {
        world.step();
        let mut flying = false;
        for entity in world.entities.iter() {
            if let Some(hook) = world.hook.get(entity) {
                flying = true;
                caught |= hook.caught.is_some();
            }
        }
        if flying {
            flew = true;
        } else if flew {
            break;
        }
    }
    assert!(flew, "it was thrown");
    assert!(!caught, "and caught nobody over open ground");
    assert!(
        world.entities.iter().all(|e| world.hook.get(e).is_none()),
        "and came back"
    );
    assert!(world.alive(pudge));
}

#[test]
fn a_hook_flies_no_further_than_it_reaches() {
    let (mut world, _pudge, _mark) = pudge_and_a_mark(4000);
    // Aimed at the very edge of its reach: further off than that the caster
    // walks in first, which is movement's business rather than the hook's.
    throw_hook(
        &mut world,
        bota_proto::Vec2::from_ints(5000 + rules::HOOK_RANGE, 5000),
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
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[1].level = 1;
    }
    let full = world.health.get(mark).expect("standing").hp;
    pudge_casts(&mut world, 1, bota_proto::Target::None);
    assert!(rotting(&world, pudge), "it is switched on");
    for _ in 0..rules::BURN_PERIOD_TICKS * 4 {
        world.step();
    }
    assert!(
        world.health.get(mark).expect("standing").hp < full,
        "what stands in it burns"
    );
    assert!(
        carries(&world, mark, crate::game::ModifierKind::Slowed { pct: 0 }),
        "and is slowed while it stands there"
    );
    pudge_casts(&mut world, 1, bota_proto::Target::None);
    assert!(!rotting(&world, pudge), "it is switched off");
    for _ in 0..3 {
        world.step();
    }
    assert!(
        !carries(&world, mark, crate::game::ModifierKind::Slowed { pct: 0 }),
        "and nothing is left slowed"
    );
}

#[test]
fn the_rot_shows_its_cloud_where_its_owner_stands_while_it_is_on() {
    // The mark stands too far off to be taken on, so he only moves when
    // moved.
    let (mut world, pudge, _mark) = pudge_and_a_mark(2000);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[1].level = 1;
    }
    pudge_casts(&mut world, 1, bota_proto::Target::None);
    let cloud = world
        .mark_of(pudge, crate::game::ability::ROT)
        .expect("switched on, the cloud shows");
    let stood = world.transform.get(pudge).expect("standing").pos;
    assert_eq!(world.transform.get(cloud).map(|t| t.pos), Some(stood));
    world.transform.get_mut(pudge).expect("standing").pos =
        stood + bota_proto::Vec2::from_ints(0, 90);
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
            .any(|shown| shown.ability == Some(crate::game::ability::ROT)),
        "and is in the view"
    );
    pudge_casts(&mut world, 1, bota_proto::Target::None);
    assert_eq!(
        world.mark_of(pudge, crate::game::ability::ROT),
        None,
        "switched off, it goes"
    );
    pudge_casts(&mut world, 1, bota_proto::Target::None);
    assert!(world.mark_of(pudge, crate::game::ability::ROT).is_some());
    let mut events = Vec::new();
    world.bury(vec![(pudge, None)], &mut events);
    assert!(
        world.entities.iter().all(|entity| world
            .mark
            .get(entity)
            .is_none_or(|shown| shown.ability != crate::game::ability::ROT)),
        "and it goes with him when he falls"
    );
}

#[test]
fn the_rot_never_kills_the_one_carrying_it() {
    let mut world = World::new();
    let pudge = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(1),
    );
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[1].level = 3;
    }
    world.settle();
    world.step();
    world.put_modifier(
        pudge,
        crate::game::Modifier {
            kind: crate::game::ModifierKind::Rot { level: 3 },
            source: Some(pudge),
            ticks_left: None,
        },
    );
    world.health.insert(
        pudge,
        Health {
            hp: Fixed::from_int(20),
        },
    );
    for _ in 0..rules::BURN_PERIOD_TICKS * 40 {
        world.step();
    }
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
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[3].level = 1;
    }
    world.health.insert(
        pudge,
        Health {
            hp: Fixed::from_int(300),
        },
    );
    world.step();
    let hurt = world.health.get(pudge).expect("standing").hp;
    let full = world.health.get(mark).expect("standing").hp;
    pudge_casts(
        &mut world,
        3,
        bota_proto::Target::Unit(crate::game::wire_id(mark)),
    );
    assert!(world.is_channelling(pudge), "it takes hold");
    for _ in 0..30 {
        world.step();
    }
    assert!(
        carries(&world, mark, crate::game::ModifierKind::Stunned),
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
    // An order of any kind lets go.
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    assert!(!world.is_channelling(pudge), "an order lets go");
}

#[test]
fn a_flesh_heap_keeps_every_enemy_hero_that_falls_near_it_and_nothing_else() {
    let (mut world, pudge, mark) = pudge_and_a_mark(200);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[2].level = 1;
    }
    let foe = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5300, 5000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    let friend = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5300),
        bota_proto::SlotId(2),
        bota_proto::HeroId(0),
    );
    let far = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 + rules::FLESH_HEAP_RANGE + 200, 5000),
        bota_proto::SlotId(3),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    let heap = |world: &World| {
        world
            .stacks
            .get(pudge)
            .map_or(0, |kept| kept.of(StackKind::FleshHeap))
    };
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    let mut events = Vec::new();
    world.bury(vec![(mark, None)], &mut events);
    world.step();
    assert_eq!(
        heap(&world),
        0,
        "a creep falling beside it feeds it nothing"
    );
    world.bury(vec![(friend, None), (far, None)], &mut events);
    world.step();
    assert_eq!(
        heap(&world),
        0,
        "neither a hero of its own side, nor an enemy hero falling too far off"
    );
    // Brought down the way a fight brings a hero down: dead before it is
    // buried.
    world.health.insert(foe, Health { hp: Fixed::ZERO });
    world.bury(vec![(foe, None)], &mut events);
    world.step();
    assert_eq!(heap(&world), 1, "an enemy hero falling beside it feeds it");
    assert_eq!(
        world.stats.get(pudge).map(|s| s.attributes.strength),
        Some(bare + rules::FLESH_HEAP_STRENGTH_PER_STACK[0]),
        "and one stack is worth its strength at the heap's first level"
    );
}

/// The heap's levels: more strength a stack, and magic resistance thickening
/// what the hero already has, multiplied and not added.
#[test]
fn a_thicker_heap_is_worth_more_a_stack_and_turns_more_magic() {
    let mut world = World::new();
    let pudge = world.spawn_hero(
        bota_proto::Team::Radiant,
        bota_proto::Vec2::from_ints(5000, 5000),
        bota_proto::SlotId(0),
        bota_proto::HeroId(1),
    );
    world.settle();
    world.step();
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    assert_eq!(
        world.stats.get(pudge).map(|s| s.magic_resist_pct),
        Some(rules::HERO_MAGIC_RESIST_PCT),
        "unlearned, the heap thickens nothing"
    );
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[2].level = 4;
    }
    let mut kept = crate::game::Stacks::default();
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
        world.stats.get(pudge).map(|s| s.magic_resist_pct),
        Some(100 - (100 - base) * (100 - heap) / 100),
        "and the resistance multiplies with the hero's own"
    );
}

#[test]
fn a_flesh_heap_outlives_the_death_of_the_one_carrying_it() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(200);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[2].level = 1;
    }
    let foe = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5100, 5200),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.settle();
    world.step();
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    let mut events = Vec::new();
    world.bury(vec![(foe, None)], &mut events);
    world.step();
    assert_eq!(
        world
            .stacks
            .get(pudge)
            .map(|kept| kept.of(StackKind::FleshHeap)),
        Some(1),
        "one death has fed it"
    );
    world.bury(vec![(pudge, None)], &mut events);
    assert_eq!(world.seats[0].unit, None, "the body is gone");
    for _ in 0..world.seats[0].respawn_left {
        world.step();
    }
    let back = world.seats[0].unit.expect("stands again");
    assert_eq!(
        world
            .stacks
            .get(back)
            .map(|kept| kept.of(StackKind::FleshHeap)),
        Some(1),
        "and what it kept comes back with it"
    );
    assert_eq!(
        world.stats.get(back).map(|s| s.attributes.strength),
        Some(bare + rules::FLESH_HEAP_STRENGTH_PER_STACK[0]),
        "worth as much strength as it was before"
    );
}

#[test]
fn a_hook_flies_out_on_a_chain_of_links_and_takes_them_home() {
    let (mut world, _pudge, _mark) = pudge_and_a_mark(600);
    throw_hook(&mut world, bota_proto::Vec2::from_ints(4000, 5000));
    world.step();
    let chained = |world: &World| {
        world
            .view_full()
            .projectiles
            .iter()
            .filter(|shown| shown.ability == Some(crate::game::ability::MEAT_HOOK))
            .count()
    };
    assert_eq!(
        chained(&world),
        rules::HOOK_LINKS + 1,
        "the hook and every link of its chain are in the view"
    );
    for _ in 0..200 {
        world.step();
        if world.entities.iter().all(|e| world.hook.get(e).is_none()) {
            break;
        }
    }
    assert_eq!(chained(&world), 0, "the chain went home with the hook");
}

#[test]
fn a_dismember_shows_its_hold_on_what_it_eats_until_it_lets_go() {
    let (mut world, pudge, mark) = pudge_and_a_mark(100);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[3].level = 1;
    }
    pudge_casts(
        &mut world,
        3,
        bota_proto::Target::Unit(crate::game::wire_id(mark)),
    );
    let shown = world
        .mark_of(pudge, crate::game::ability::DISMEMBER)
        .expect("the hold is shown");
    let eaten = world.transform.get(mark).expect("standing").pos;
    assert_eq!(world.transform.get(shown).map(|t| t.pos), Some(eaten));
    world.transform.get_mut(mark).expect("standing").pos =
        eaten + bota_proto::Vec2::from_ints(40, 0);
    world.step();
    assert_eq!(
        world.transform.get(shown).map(|t| t.pos),
        world.transform.get(mark).map(|t| t.pos),
        "and follows what is eaten"
    );
    world.advance(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
    }]);
    assert_eq!(
        world.mark_of(pudge, crate::game::ability::DISMEMBER),
        None,
        "letting go takes the hold away"
    );
}

#[test]
fn a_structure_falling_beside_a_flesh_heap_feeds_it_nothing() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(200);
    if let Some(book) = world.abilities.get_mut(pudge) {
        book.slots[2].level = 1;
    }
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5200, 5000),
    );
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(tower, None)], &mut events);
    assert_eq!(
        world
            .stacks
            .get(pudge)
            .map_or(0, |kept| kept.of(StackKind::FleshHeap)),
        0,
    );
}
