//! Pudge.

use bota_core::{ModifierKind, rules, tower_def};

use crate::game::{Entity, Modifier, World, wire_id};
use bota_proto::{
    AbilityId, EffectId, Fixed, HeroId, Inflictor, SlotId, StatusFlags, Target, Team, Vec2,
};

use super::support::*;

#[test]
fn a_hook_catches_what_it_flies_into_and_drags_it_home() {
    let (mut world, pudge, mark) = pudge_and_a_mark(600);
    let full = world.health.get(mark).expect("standing").hp;
    let home = world.transform.get(pudge).expect("standing").pos;
    cast(&mut world, 0, Target::Pos(Vec2::from_ints(6000, 5000)));
    until_the_hook_flies(&mut world);
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
    until_the_hook_flies(&mut world);
    let chained = |world: &World| {
        world
            .view_full()
            .projectiles
            .iter()
            .filter(|shown| shown.inflictor == Some(Inflictor::Ability(AbilityId::MEAT_HOOK)))
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
        carries(&world, mark, ModifierKind::Rotting { level: 0 }),
        "and is slowed while it stands there"
    );
    cast(&mut world, 1, Target::None);
    assert!(
        !carries(&world, pudge, ModifierKind::Rot { level: 0 }),
        "it is switched off"
    );
    run(&mut world, 3);
    assert!(
        !carries(&world, mark, ModifierKind::Rotting { level: 0 }),
        "and nothing is left slowed"
    );
}

#[test]
fn the_rot_shows_as_one_rot_on_its_owner_and_on_what_stands_in_it() {
    // Dota shows modifier_pudge_rot, "Rot", on Pudge and on the enemies in
    // it: one icon each, not a burn and a slow, and the toggle is on while
    // Pudge wears the rot he put on.
    let (mut world, pudge, mark) = pudge_and_a_mark(150);
    set_level(&mut world, pudge, 1, 1);
    cast(&mut world, 1, Target::None);
    run(&mut world, 2);
    let rot = Some(Inflictor::Ability(AbilityId::ROT));
    let view = world.view_full();
    let shown = |unit| {
        view.units
            .iter()
            .find(|shown| shown.id == wire_id(unit))
            .expect("in the view")
    };
    let own: Vec<_> = shown(pudge)
        .effects
        .iter()
        .filter(|e| e.inflictor == rot)
        .collect();
    assert_eq!(own.len(), 1, "one rot on Pudge: {own:?}");
    assert_eq!(own[0].id, EffectId::ROT);
    assert_eq!(own[0].caster, Some(wire_id(pudge)));
    assert_eq!(own[0].ticks_left, None, "a toggle runs until switched off");
    let theirs: Vec<_> = shown(mark)
        .effects
        .iter()
        .filter(|e| e.inflictor == rot)
        .collect();
    assert_eq!(theirs.len(), 1, "one rot on what stands in it: {theirs:?}");
    assert_eq!(theirs[0].id, EffectId::ROT);
    assert_eq!(theirs[0].caster, Some(wire_id(pudge)));
    let bits = shown(mark).statuses.bits;
    assert_eq!(
        bits & (StatusFlags::SLOWED | StatusFlags::DOT),
        StatusFlags::SLOWED | StatusFlags::DOT,
        "it slows and burns"
    );
    assert!(
        shown(pudge)
            .abilities
            .iter()
            .any(|ability| ability.id == AbilityId::ROT && ability.on),
        "the toggle shows on"
    );
    cast(&mut world, 1, Target::None);
    run(&mut world, 3);
    let view = world.view_full();
    let pudge_view = view
        .units
        .iter()
        .find(|shown| shown.id == wire_id(pudge))
        .expect("in the view");
    assert!(
        pudge_view
            .abilities
            .iter()
            .any(|ability| ability.id == AbilityId::ROT && !ability.on),
        "and off again"
    );
    assert!(pudge_view.effects.iter().all(|e| e.inflictor != rot));
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
            .any(|shown| shown.inflictor == Some(Inflictor::Ability(AbilityId::ROT))),
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
        Modifier::new(ModifierKind::Rot { level: 3 }, Some(pudge), None, None),
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
    dismember(&mut world, mark);
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
    dismember(&mut world, mark);
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
        Some(bare + rules::FLESH_HEAP_STRENGTH_PER_STACK),
        "and one stack is worth its strength"
    );
}

/// A stack is worth two strength flat, whatever the hero's level, and the heap
/// leaves magic resistance alone.
#[test]
fn a_stack_of_the_heap_is_worth_two_strength_and_nothing_else() {
    let (mut world, pudge) = a_lone_pudge();
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    let resist = world.stats.get(pudge).map(|s| s.magic_resist_bp);
    world
        .modifiers
        .get_mut(pudge)
        .expect("a hero wears modifiers")
        .set_count(ModifierKind::flesh_heap, 3, (pudge, AbilityId::FLESH_HEAP));
    world.step();
    assert_eq!(
        world.stats.get(pudge).map(|s| s.attributes.strength),
        Some(bare + Fixed::from_int(6)),
        "three stacks are worth two strength each"
    );
    assert_eq!(world.stats.get(pudge).map(|s| s.magic_resist_bp), resist);
}

#[test]
fn a_flesh_heap_outlives_the_death_of_the_one_carrying_it() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(200);
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
        Some(bare + rules::FLESH_HEAP_STRENGTH_PER_STACK),
        "worth as much strength as it was before"
    );
}

/// Steps until the hook has left Pudge: its cast point is over.
fn until_the_hook_flies(world: &mut World) {
    for _ in 0..rules::HOOK_CAST_POINT_MS / 30 + 2 {
        if hook_flying(world) {
            return;
        }
        world.step();
    }
    assert!(hook_flying(world), "the hook leaves after its cast point");
}

/// Orders a dismember and steps until its cast point is over and it holds.
fn dismember(world: &mut World, mark: Entity) {
    cast(world, 3, Target::Unit(wire_id(mark)));
    for _ in 0..rules::DISMEMBER_CAST_POINT_MS / 30 + 2 {
        if mark_is_held(world, mark) {
            return;
        }
        world.step();
    }
    assert!(
        mark_is_held(world, mark),
        "it takes hold after its cast point"
    );
}

fn mark_is_held(world: &World, mark: Entity) -> bool {
    carries(world, mark, ModifierKind::Stunned)
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

fn heap(world: &World, hero: Entity) -> u32 {
    world
        .modifiers
        .get(hero)
        .map_or(0, |on_it| on_it.count_of(ModifierKind::flesh_heap))
}

#[test]
fn a_hook_costs_120_mana_waits_18_seconds_and_deals_150_pure_damage_at_its_first_level() {
    let (mut world, pudge, mark) = pudge_and_a_mark(600);
    let full = world.health.get(mark).expect("standing").hp.to_int();
    world.fill_pools(pudge);
    let mana = world.mana.get(pudge).expect("has mana").mana;
    cast(&mut world, 0, Target::Pos(Vec2::from_ints(5600, 5000)));
    until_the_hook_flies(&mut world);
    assert_eq!(
        mana - world.mana.get(pudge).expect("has mana").mana,
        Fixed::from_int(120),
        "the hook costs 120 mana"
    );
    for _ in 0..200 {
        world.step();
        if !hook_flying(&world) {
            break;
        }
    }
    assert_eq!(
        world.health.get(mark).expect("standing").hp.to_int(),
        full - 150,
        "an enemy takes 150, whatever armor it wears"
    );
    let waiting = world.abilities.get(pudge).expect("a kit").slots[0].cooldown;
    assert!(
        (540 - 40..=540).contains(&waiting),
        "and the next hook is 18 seconds off, less the flight: {waiting} ticks"
    );
}

/// The dismember bites six times, the first as it takes hold: each bite is
/// the level's damage a second and 0.3 a point of strength, over 2.75 seconds
/// shared six ways, and the one holding mends by what each takes. It pulls
/// what it holds to 125 units away at 75 a second.
#[test]
fn a_dismember_bites_six_times_by_strength_pulls_and_heals_by_what_it_takes() {
    let (mut world, pudge, mark) = pudge_and_a_mark(190);
    set_level(&mut world, pudge, 3, 1);
    world.fill_pools(pudge);
    world.health.insert(pudge, health(300));
    world.step();
    let strength = world.stats.get(pudge).expect("settled").attributes.strength;
    let bite = bota_core::dismember_bite_damage(0, strength);
    assert_eq!(
        bite,
        ((Fixed::from_int(80) + strength * Fixed::from_ratio(3, 10))
            * Fixed::from_ratio(2750, 6000))
        .to_int(),
        "a bite is the second's damage over 2.75 seconds in six"
    );
    let start = world.health.get(pudge).expect("standing").hp.to_int();
    let mut left = world.health.get(mark).expect("standing").hp.to_int();
    let mut bites = Vec::new();
    cast(&mut world, 3, Target::Unit(wire_id(mark)));
    for tick in 0..rules::DISMEMBER_TICKS + 5 {
        let now = world.health.get(mark).expect("standing").hp.to_int();
        // Its own regeneration may give back a point between two readings.
        if left - now >= bite - 1 {
            assert!(left - now <= bite, "each bite takes the same");
            bites.push(tick);
        }
        left = now;
        world.step();
    }
    assert_eq!(
        bites.len(),
        rules::DISMEMBER_BITES as usize,
        "it bites six times in all"
    );
    assert_eq!(
        bites[0],
        ticks_of(rules::DISMEMBER_CAST_POINT_MS),
        "the first as it takes hold, once the cast point is over"
    );
    let gaps: Vec<u32> = bites.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert_eq!(gaps, [14, 14, 14, 13, 14], "evenly over 2.75 seconds");
    let healed = world.health.get(pudge).expect("standing").hp.to_int() - start;
    let regen = world.stats.get(pudge).expect("settled").hp_regen
        * Fixed::from_int((rules::DISMEMBER_TICKS + 5) as i32);
    assert!(
        (bite * 6..=bite * 6 + regen.to_int() + 1).contains(&healed),
        "and the one eating mends by all of it, his own regeneration aside: {healed}"
    );
    let apart = world.transform.get(mark).expect("standing").pos
        - world.transform.get(pudge).expect("standing").pos;
    assert!(
        (124..=126).contains(&apart.x.to_int()),
        "what it held was pulled in to 125: {apart:?}"
    );
}

#[test]
fn a_meat_shield_takes_its_block_off_every_blow_for_four_seconds() {
    let (mut world, pudge, _mark) = pudge_and_a_mark(2000);
    set_level(&mut world, pudge, 2, 1);
    world.fill_pools(pudge);
    let mana = world.mana.get(pudge).expect("has mana").mana;
    cast(&mut world, 2, Target::None);
    world.step();
    let spent = mana - world.mana.get(pudge).expect("has mana").mana;
    assert!(
        (64..=65).contains(&spent.to_int()),
        "it costs 65 mana, less a tick of regeneration: {spent}"
    );
    let hp = |world: &World| world.health.get(pudge).expect("standing").hp.to_int();
    let before = hp(&world);
    world.push_hit(None, pudge, 30, bota_proto::DamageKind::Pure, None);
    world.push_hit(None, pudge, 5, bota_proto::DamageKind::Pure, None);
    world.step();
    assert_eq!(
        before - hp(&world),
        30 - 8,
        "every blow loses 8: a blow of 30 costs 22 and one of 5 costs nothing"
    );
    run(&mut world, 4 * rules::TICKS_PER_SECOND);
    let before = hp(&world);
    world.push_hit(None, pudge, 30, bota_proto::DamageKind::Pure, None);
    world.step();
    assert!(
        before - hp(&world) >= 29,
        "and after four seconds it is gone (a point of regeneration aside)"
    );
}
