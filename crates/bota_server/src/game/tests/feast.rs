//! Shadow Fiend's feast of souls.

use bota_core::{ModifierKind, rules};

use crate::game::{Entity, World, wire_id};
use bota_proto::{AbilityId, EffectId, EffectView, HeroId, Inflictor, SlotId, Target, Team, Vec2};

use super::support::*;

const FEAST_SLOT: u8 = 3;

/// Shadow Fiend with every slot at its first level, an enemy hero 300 east,
/// three enemy creeps 200, 400 and 500 east and one 800 east out of reach,
/// every enemy stunned where it stands and the fiend told to swing at
/// nobody.
fn a_feast_table() -> (World, Entity, Entity, [Entity; 4]) {
    let (mut world, fiend, near) = fiend_and_a_mark(200);
    let hero = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5300, 5000),
        SlotId(1),
        HeroId::SHADOW_FIEND,
    );
    let mid = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(5400, 5000));
    let far = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(5500, 5000));
    let out = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(5800, 5000));
    for foe in [hero, near, mid, far, out] {
        put_on(&mut world, foe, ModifierKind::Stunned, 10_000);
    }
    world.settle();
    stop_click(&mut world);
    (world, fiend, hero, [near, mid, far, out])
}

#[test]
fn the_feast_hastens_its_caster_at_once_and_for_eight_seconds() {
    let (mut world, fiend, _hero, _creeps) = a_feast_table();
    let before = *world.stats.get(fiend).expect("settled");
    cast(&mut world, FEAST_SLOT, Target::None);
    world.step();
    let during = *world.stats.get(fiend).expect("settled");
    assert_eq!(during.attack_speed, before.attack_speed + 35);
    assert_eq!(
        during.move_speed.raw,
        (i64::from(before.move_speed.raw) * 104 / 100) as i32
    );
    assert_eq!(
        world.abilities.get(fiend).expect("a kit").slots[usize::from(FEAST_SLOT)].cooldown,
        rules::FEAST_COOLDOWN[0] - 1,
        "21 s on the clock from the cast"
    );
    run(&mut world, rules::FEAST_TICKS - 1);
    assert_eq!(
        world.stats.get(fiend).expect("settled").attack_speed,
        before.attack_speed,
        "and gone once eight seconds have run"
    );
}

#[test]
fn the_feast_takes_souls_from_two_enemies_every_half_second_heroes_first_up_to_its_level() {
    let (mut world, fiend, _hero, _creeps) = a_feast_table();
    cast(&mut world, FEAST_SLOT, Target::None);
    let mut taken = Vec::new();
    for tick in 1..=60 {
        world.step();
        if tick % 15 == 0 {
            taken.push(souls(&world, fiend));
        }
    }
    // The hero's three and the nearest creep's one, then two more creeps,
    // then nothing: four enemies at the first level, the one out of reach
    // never.
    assert_eq!(taken, vec![4, 6, 6, 6]);
    assert_eq!(
        feast_effect(&world, fiend),
        Some(EffectView {
            id: EffectId::FEASTING,
            ticks_left: Some(rules::FEAST_TICKS - 60),
            stacks: Some(6),
            caster: Some(wire_id(fiend)),
            inflictor: Some(Inflictor::Ability(AbilityId::FEAST_OF_SOULS)),
            ticks_total: Some(rules::FEAST_TICKS),
        })
    );
}

#[test]
fn once_the_feast_is_over_the_souls_of_the_living_go_and_the_dead_linger_for_eight_seconds() {
    let (mut world, fiend, _hero, [near, ..]) = a_feast_table();
    cast(&mut world, FEAST_SLOT, Target::None);
    run(&mut world, 30);
    assert_eq!(souls(&world, fiend), 6);
    world.bury(vec![(near, Some(fiend))], &mut Vec::new());
    assert_eq!(
        souls(&world, fiend),
        7,
        "a kill is worth its own soul besides"
    );
    for _ in 30..rules::FEAST_TICKS {
        world.step();
    }
    // The hero's three and two creeps' two go; the dead creep's one lingers
    // beside the soul its kill was worth.
    assert_eq!(souls(&world, fiend), 2);
    assert_eq!(
        feast_effect(&world, fiend),
        Some(EffectView {
            id: EffectId::FEAST_LINGER,
            ticks_left: Some(rules::FEAST_LINGER_TICKS),
            stacks: Some(1),
            caster: Some(wire_id(fiend)),
            inflictor: Some(Inflictor::Ability(AbilityId::FEAST_OF_SOULS)),
            ticks_total: Some(rules::FEAST_LINGER_TICKS),
        })
    );
    run(&mut world, rules::FEAST_LINGER_TICKS);
    assert_eq!(feast_effect(&world, fiend), None, "the linger has run out");
    assert_eq!(souls(&world, fiend), 2, "and two are within the cap");
}

#[test]
fn souls_a_feast_carries_past_the_cap_are_lost_when_the_linger_ends() {
    let (mut world, fiend, hero, _creeps) = a_feast_table();
    hand_souls(&mut world, fiend, rules::NECRO_SOUL_CAP);
    cast(&mut world, FEAST_SLOT, Target::None);
    run(&mut world, 30);
    assert_eq!(souls(&world, fiend), 26, "past the cap while it runs");
    world.bury(vec![(hero, Some(fiend))], &mut Vec::new());
    assert_eq!(souls(&world, fiend), 26, "a kill adds nothing past the cap");
    for _ in 30..rules::FEAST_TICKS {
        world.step();
    }
    assert_eq!(
        souls(&world, fiend),
        23,
        "the three living creeps' souls go"
    );
    run(&mut world, rules::FEAST_LINGER_TICKS);
    assert_eq!(souls(&world, fiend), rules::NECRO_SOUL_CAP);
}

#[test]
fn a_raze_in_a_feast_burns_for_every_soul_the_feast_has_taken() {
    let (mut world, fiend, _hero, [near, ..]) = a_feast_table();
    cast(&mut world, FEAST_SLOT, Target::None);
    run(&mut world, 15);
    assert_eq!(souls(&world, fiend), 4);
    let events = cast_through(&mut world, 0, rules::RAZE_CAST_POINT_MS);
    assert_eq!(burns_on(&events, near), vec![85 + 2 * 6]);
}

#[test]
fn the_feast_goes_off_beside_a_raze_without_breaking_its_cast_point() {
    let (mut world, fiend, _hero, [near, ..]) = a_feast_table();
    cast(&mut world, 0, Target::None);
    run(&mut world, 5);
    cast(&mut world, FEAST_SLOT, Target::None);
    assert!(feast_effect(&world, fiend).is_some(), "the feast is on");
    let mut burnt = Vec::new();
    for _ in 0..12 {
        burnt.extend(burns_on(&world.step(), near));
    }
    assert_eq!(
        burnt,
        vec![rules::RAZE_DAMAGE[0]],
        "and the raze lands on time"
    );
}

#[test]
fn a_fiend_falling_mid_feast_loses_the_feasts_souls_before_his_share() {
    let (mut world, fiend, _hero, _creeps) = a_feast_table();
    hand_souls(&mut world, fiend, 10);
    cast(&mut world, FEAST_SLOT, Target::None);
    run(&mut world, 30);
    assert_eq!(souls(&world, fiend), 16);
    world.bury(vec![(fiend, None)], &mut Vec::new());
    let back = back_on_its_feet(&mut world);
    assert_eq!(
        souls(&world, back),
        7,
        "the feast's six go, then three of ten"
    );
    assert_eq!(feast_effect(&world, back), None);
}

fn feast_effect(world: &World, unit: Entity) -> Option<EffectView> {
    world
        .view_full()
        .units
        .into_iter()
        .find(|shown| shown.id == wire_id(unit))?
        .effects
        .into_iter()
        .find(|effect| [EffectId::FEASTING, EffectId::FEAST_LINGER].contains(&effect.id))
}
