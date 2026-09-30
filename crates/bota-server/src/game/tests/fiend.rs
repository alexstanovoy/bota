//! Shadow Fiend.

use crate::game::rules;
use crate::game::{ModifierKind, StackKind, World};
use bota_proto::Fixed;

use super::support::*;

#[test]
fn a_raze_burns_what_stands_where_it_lands_and_nothing_else() {
    let (mut world, _fiend, near) = fiend_and_a_mark(rules::RAZE_DISTANCE[1]);
    let far = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 + 900, 5000),
    );
    let (was_near, was_far) = (
        world.health.get(near).expect("standing").hp,
        world.health.get(far).expect("standing").hp,
    );
    let_go(&mut world, 1);
    world.step();
    assert!(
        world.health.get(near).expect("standing").hp < was_near,
        "what stands where the raze lands feels it"
    );
    assert_eq!(
        world.health.get(far).expect("standing").hp,
        was_far,
        "and what stands past it does not"
    );
}

#[test]
fn a_raze_lands_at_its_own_reach_however_near_the_enemy_stands() {
    let (mut world, _fiend, under) = fiend_and_a_mark(50);
    let out = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 + rules::RAZE_DISTANCE[2], 5000),
    );
    let (was_under, was_out) = (
        world.health.get(under).expect("standing").hp,
        world.health.get(out).expect("standing").hp,
    );
    // The farthest raze lands at its own reach, over the head of what stands
    // right under his feet.
    let_go(&mut world, 2);
    world.step();
    assert!(
        world.health.get(out).expect("standing").hp < was_out,
        "the raze lands at its own reach"
    );
    assert_eq!(
        world.health.get(under).expect("standing").hp,
        was_under,
        "and nowhere nearer"
    );
}

#[test]
fn a_raze_lays_itself_along_the_facing() {
    let (mut world, _fiend, ahead) = fiend_and_a_mark(rules::RAZE_DISTANCE[1]);
    let behind = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 - rules::RAZE_DISTANCE[1], 5000),
    );
    let (was_ahead, was_behind) = (
        world.health.get(ahead).expect("standing").hp,
        world.health.get(behind).expect("standing").hp,
    );
    let_go(&mut world, 1);
    world.step();
    assert!(
        world.health.get(ahead).expect("standing").hp < was_ahead,
        "what stands where he faces feels it"
    );
    assert_eq!(
        world.health.get(behind).expect("standing").hp,
        was_behind,
        "and what stands behind him does not"
    );
}

#[test]
fn souls_come_only_from_what_the_gatherer_brings_down() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let mut events = Vec::new();
    let other = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5000),
    );
    world.bury(vec![(other, None)], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map_or(0, |kept| kept.of(StackKind::Souls)),
        0,
        "a death nobody is answerable for is worth nothing"
    );
    world.bury(vec![(mark, Some(fiend))], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map(|kept| kept.of(StackKind::Souls)),
        Some(rules::SOULS_PER_UNIT),
        "and one he brought down is worth its soul"
    );
}

#[test]
fn a_hero_brought_down_is_worth_more_souls_than_a_creep() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    let victim = world.spawn_hero(
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5400, 5000),
        bota_proto::SlotId(1),
        bota_proto::HeroId(0),
    );
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(victim, Some(fiend))], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map(|kept| kept.of(StackKind::Souls)),
        Some(rules::SOULS_PER_HERO),
    );
}

#[test]
fn souls_stop_at_what_the_necromastery_holds() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let cap = world.soul_cap(fiend);
    assert_eq!(
        cap,
        rules::NECRO_SOUL_CAP[0],
        "the necromastery at its first level"
    );
    hand_souls(&mut world, fiend, cap);
    let mut events = Vec::new();
    world.bury(vec![(mark, Some(fiend))], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map(|kept| kept.of(StackKind::Souls)),
        Some(cap),
        "no more are held than the level allows"
    );
}

#[test]
fn every_soul_held_is_worth_attack_damage() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    world.step();
    let bare = world.stats.get(fiend).expect("settled").damage;
    hand_souls(&mut world, fiend, 5);
    world.step();
    assert_eq!(
        world.stats.get(fiend).map(|s| s.damage),
        Some(bare + rules::DAMAGE_PER_SOUL * 5),
    );
}

#[test]
fn a_raze_goes_off_where_it_is_asked_for_and_walks_the_caster_nowhere() {
    // Near enough for the shortest raze, far enough that the bodies do not
    // touch and get eased apart.
    let (mut world, fiend, _mark) = fiend_and_a_mark(80);
    let out = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(6000, 5000),
    );
    let stood = world.transform.get(fiend).expect("standing").pos;
    let full = world.health.get(out).expect("standing").hp;
    let_go(&mut world, 1);
    world.step();
    assert_eq!(
        world.transform.get(fiend).expect("standing").pos,
        stood,
        "a raze takes no aim, so there is nothing to walk into"
    );
    assert_eq!(
        world.health.get(out).expect("standing").hp,
        full,
        "and what stands past its reach is missed, not chased"
    );
    assert!(
        world
            .abilities
            .get(fiend)
            .is_some_and(|book| book.slots[1].cooldown > 0),
        "the cast itself went off"
    );
}

#[test]
fn the_presence_wears_down_the_armor_of_enemies_near_its_carrier() {
    let (mut world, _fiend, near) = fiend_and_a_mark(400);
    let out = a_creep_at(
        &mut world,
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5000 + rules::PRESENCE_RADIUS + 500, 5000),
    );
    world.step();
    let (worn, whole) = (
        world.stats.get(near).expect("standing").armor,
        world.stats.get(out).expect("standing").armor,
    );
    assert_eq!(
        worn,
        whole - Fixed::from_int(rules::PRESENCE_ARMOR[0]),
        "standing in the presence costs its armor"
    );
}

#[test]
fn souls_outlive_the_death_of_the_one_who_gathered_them() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let mut events = Vec::new();
    world.bury(vec![(mark, Some(fiend))], &mut events);
    world.bury(vec![(fiend, None)], &mut events);
    assert_eq!(world.seats[0].unit, None, "the body is gone");
    for _ in 0..world.seats[0].respawn_left {
        world.step();
    }
    let back = world.seats[0].unit.expect("stands again");
    assert_eq!(
        world.stacks.get(back).map(|kept| kept.of(StackKind::Souls)),
        Some(rules::SOULS_PER_UNIT),
        "what was gathered comes back with him"
    );
}

#[test]
fn a_requiem_lets_a_line_fly_for_every_soul_and_a_line_burns_what_it_crosses_once() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    hand_souls(&mut world, fiend, 5);
    world.step();
    let full = world.health.get(mark).expect("standing").hp;
    let_go(&mut world, 5);
    let flying = |world: &World| {
        world
            .view_full()
            .projectiles
            .iter()
            .filter(|shown| shown.ability == Some(crate::game::ability::REQUIEM))
            .count()
    };
    assert_eq!(flying(&world), 5, "one line to a soul");
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map(|kept| kept.of(StackKind::Souls)),
        Some(5),
        "and the souls are kept"
    );
    // The line along the fiend's facing reaches the mark; the others fly
    // wide of it. His swings at it are not the requiem's and are left out.
    let mut burns = Vec::new();
    for _ in 0..60 {
        for event in world.step() {
            if let bota_proto::EventKind::Damaged {
                source,
                target,
                amount,
                kind: bota_proto::DamageKind::Magical,
                ..
            } = event.kind
                && source == Some(crate::game::wire_id(fiend))
                && target == crate::game::wire_id(mark)
            {
                burns.push(amount);
            }
        }
    }
    assert_eq!(
        burns,
        vec![rules::REQUIEM_LINE_DAMAGE[0]],
        "one line's worth, once"
    );
    assert!(
        world.health.get(mark).expect("standing").hp < full,
        "and it was felt"
    );
    assert_eq!(flying(&world), 0, "and the lines have flown out");
}

#[test]
fn what_stands_at_the_fiend_is_crossed_by_every_line_and_held_the_longest() {
    let (mut world, fiend, mark) = fiend_and_a_mark(60);
    hand_souls(&mut world, fiend, 5);
    world.step();
    let full = world.health.get(mark).expect("standing").hp;
    let_go(&mut world, 5);
    world.step();
    let taken = full - world.health.get(mark).expect("standing").hp;
    assert_eq!(
        taken,
        Fixed::from_int(5 * rules::REQUIEM_LINE_DAMAGE[0]),
        "five lines, five burns"
    );
    let held = world.modifiers.get(mark).and_then(|on_it| {
        on_it
            .active()
            .find(|held| held.kind == ModifierKind::Feared && held.source == Some(fiend))
            .and_then(|held| held.ticks_left)
    });
    assert_eq!(
        held,
        Some(rules::REQUIEM_HOLD_MAX_TICKS),
        "five lines' fear, capped"
    );
    assert!(
        world.modifiers.get(mark).is_some_and(|on_it| on_it
            .active()
            .any(|held| matches!(held.kind, ModifierKind::Slowed { .. }))),
        "and it walks slower"
    );
    let projected = world
        .view(bota_proto::Team::Dire)
        .units
        .into_iter()
        .find(|unit| unit.id == crate::game::wire_id(mark))
        .expect("the frightened creep is projected");
    assert_ne!(
        projected.statuses.bits & bota_proto::StatusFlags::FEARED,
        0,
        "and the fear is on the wire"
    );
}

#[test]
fn what_the_requiem_frightens_runs_from_the_fiend_and_swings_at_nothing() {
    let (mut world, fiend, mark) = fiend_and_a_mark(150);
    hand_souls(&mut world, fiend, 5);
    world.step();
    let apart = |world: &World| {
        crate::game::isqrt64(
            world
                .transform
                .get(fiend)
                .expect("standing")
                .pos
                .distance_squared(world.transform.get(mark).expect("standing").pos),
        )
    };
    let_go(&mut world, 5);
    world.step();
    assert!(
        world.feared(mark),
        "the first line crossing it frightens it"
    );
    let before = apart(&world);
    for _ in 0..10 {
        world.step();
        assert!(
            !matches!(
                world.action.get(mark).map(|action| action.state),
                Some(crate::game::ActionState::Attack { .. })
            ),
            "feared, it swings at nothing"
        );
    }
    let after = apart(&world);
    // Ten ticks, a few of them spent turning round.
    assert!(
        after > before + Fixed::from_int(50).raw as i64,
        "and it has run: {before} then {after}"
    );
}

#[test]
fn a_death_lets_a_share_of_the_souls_go() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    hand_souls(&mut world, fiend, 10);
    let mut events = Vec::new();
    world.bury(vec![(fiend, None)], &mut events);
    for _ in 0..world.seats[0].respawn_left {
        world.step();
    }
    let back = world.seats[0].unit.expect("stands again");
    assert_eq!(
        world.stacks.get(back).map(|kept| kept.of(StackKind::Souls)),
        Some(7),
        "three of ten are let go"
    );
}

#[test]
fn a_raze_leaves_its_mark_where_it_lands_for_whoever_sees_the_spot() {
    // The creep stands past its own sight of the fiend, and the far raze
    // lands beside it.
    let apart = rules::RAZE_DISTANCE[2] + 150;
    assert!(
        apart > rules::CREEP_VISION,
        "the fiend is out of the creep's sight"
    );
    let (mut world, _fiend, _mark) = fiend_and_a_mark(apart);
    let_go(&mut world, 2);
    world.step();
    let dire = world.view(bota_proto::Team::Dire);
    let landed = dire
        .projectiles
        .iter()
        .find(|shown| shown.ability == Some(crate::game::ability::RAZE_FAR))
        .expect("the Dire side sees the raze where it landed");
    assert_eq!(
        landed.pos,
        bota_proto::Vec2::from_ints(5000 + rules::RAZE_DISTANCE[2], 5000)
    );
    assert!(
        dire.units
            .iter()
            .all(|shown| shown.hero != Some(bota_proto::HeroId(2))),
        "and still does not see the fiend"
    );
    for _ in 0..rules::MARK_TICKS {
        world.step();
    }
    assert!(
        world
            .view_full()
            .projectiles
            .iter()
            .all(|shown| shown.ability != Some(crate::game::ability::RAZE_FAR)),
        "the mark is gone once its ticks have run"
    );
}

#[test]
fn a_requiem_with_no_souls_gathered_touches_nobody() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let full = world.health.get(mark).expect("standing").hp;
    let_go(&mut world, 5);
    world.step();
    world.step();
    assert_eq!(
        world.health.get(mark).expect("standing").hp,
        full,
        "nothing gathered is nothing let go"
    );
    assert!(
        world.modifiers.get(mark).is_none_or(|on_it| on_it
            .active()
            .all(|held| !matches!(held.kind, ModifierKind::Slowed { .. }))),
        "not even the slow"
    );
    let spent = world
        .abilities
        .get(fiend)
        .map_or(0, |book| book.slots[5].cooldown);
    assert!(spent > 0, "though the cast itself happened");
}

#[test]
fn a_structure_brought_down_is_worth_no_soul() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    let tower = world.spawn_unit(
        crate::game::tower_def(1),
        bota_proto::Team::Dire,
        bota_proto::Vec2::from_ints(5600, 5000),
    );
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(tower, Some(fiend))], &mut events);
    assert_eq!(
        world
            .stacks
            .get(fiend)
            .map_or(0, |kept| kept.of(StackKind::Souls)),
        0,
    );
}

#[test]
fn the_view_carries_what_has_been_gathered_as_a_counted_effect() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let mut events = Vec::new();
    world.bury(vec![(mark, Some(fiend))], &mut events);
    world.step();
    let view = world.view(bota_proto::Team::Radiant);
    let shown = view
        .units
        .iter()
        .find(|u| u.hero == Some(bota_proto::HeroId(2)))
        .expect("he is in the view");
    let souls = shown
        .effects
        .iter()
        .find(|e| e.stacks.is_some())
        .expect("what is gathered is on him");
    assert_eq!(souls.stacks, Some(rules::SOULS_PER_UNIT));
    assert_eq!(souls.ticks_left, None, "and nothing counts it down");
}
