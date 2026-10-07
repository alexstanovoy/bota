//! Shadow Fiend.

use crate::game::{
    ActionState, Entity, ModifierKind, StackKind, World, ability, isqrt64, rules, tower_def,
    wire_id,
};
use bota_proto::{DamageKind, EventKind, Fixed, HeroId, SlotId, StatusFlags, Team, Vec2};

use super::support::*;

#[test]
fn a_raze_burns_what_stands_where_it_lands_and_nothing_else() {
    let mid = rules::RAZE_DISTANCE[1];
    // The mark stands `apart` east, another creep `other` east; one of them
    // stands where the raze lands.
    for (case, apart, other, slot, mark_struck) in [
        ("a creep past it", mid, 900, 1, true),
        ("one under his feet", 50, rules::RAZE_DISTANCE[2], 2, false),
        ("one behind him", mid, -mid, 1, true),
    ] {
        let (mut world, _fiend, mark) = fiend_and_a_mark(apart);
        let other = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(5000 + other, 5000));
        let (struck, spared) = if mark_struck {
            (mark, other)
        } else {
            (other, mark)
        };
        let (was_struck, was_spared) = (hp(&world, struck), hp(&world, spared));
        let_go(&mut world, slot);
        world.step();
        assert!(
            hp(&world, struck) < was_struck,
            "{case}: what stands where it lands feels it"
        );
        assert_eq!(hp(&world, spared), was_spared, "{case}: is spared");
    }
}

#[test]
fn a_raze_or_a_requiem_burns_the_creep_at_a_tower_and_leaves_the_tower_whole() {
    let mid = rules::RAZE_DISTANCE[1];
    for (case, slot, apart, tower_at, ticks) in [
        ("raze", 1, mid, Vec2::from_ints(5000 + mid, 5200), 1),
        ("requiem", 5, 400, Vec2::from_ints(5700, 5000), 60),
    ] {
        let (mut world, fiend, creep) = fiend_and_a_mark(apart);
        let tower = world.spawn_unit(tower_def(1), Team::Dire, tower_at);
        world.settle();
        hand_souls(&mut world, fiend, 5);
        world.step();
        let (was_creep, was_tower) = (hp(&world, creep), hp(&world, tower));
        let_go(&mut world, slot);
        for _ in 0..ticks {
            world.step();
        }
        assert!(
            hp(&world, creep) < was_creep,
            "{case}: the creep at the tower feels it"
        );
        assert_eq!(hp(&world, tower), was_tower, "{case}: the tower does not");
        assert!(
            world
                .modifiers
                .get(tower)
                .is_none_or(|on_it| on_it.active().all(|held| held.source != Some(fiend))),
            "{case}: nor does it keep a stack, a fear or a slow"
        );
    }
}

#[test]
fn souls_come_only_from_the_units_the_gatherer_brings_down() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let other = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(5400, 5000));
    let tower = world.spawn_unit(tower_def(1), Team::Dire, Vec2::from_ints(5600, 5000));
    world.settle();
    let mut events = Vec::new();
    world.bury(vec![(other, None)], &mut events);
    assert_eq!(
        souls(&world, fiend),
        0,
        "a death nobody is answerable for is worth nothing"
    );
    world.bury(vec![(tower, Some(fiend))], &mut events);
    assert_eq!(souls(&world, fiend), 0, "nor is a structure");
    world.bury(vec![(mark, Some(fiend))], &mut events);
    assert_eq!(
        souls(&world, fiend),
        rules::SOULS_PER_UNIT,
        "and a creep he brought down is worth its soul"
    );
}

#[test]
fn a_hero_brought_down_is_worth_more_souls_than_a_creep() {
    let (mut world, fiend, _mark) = fiend_and_a_mark(400);
    let victim = world.spawn_hero(
        Team::Dire,
        Vec2::from_ints(5400, 5000),
        SlotId(1),
        HeroId(0),
    );
    world.settle();
    world.bury(vec![(victim, Some(fiend))], &mut Vec::new());
    assert_eq!(souls(&world, fiend), rules::SOULS_PER_HERO);
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
    world.bury(vec![(mark, Some(fiend))], &mut Vec::new());
    assert_eq!(
        souls(&world, fiend),
        cap,
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
    let out = a_creep_at(&mut world, Team::Dire, Vec2::from_ints(6000, 5000));
    let stood = world.transform.get(fiend).expect("standing").pos;
    let full = hp(&world, out);
    let_go(&mut world, 1);
    world.step();
    assert_eq!(
        world.transform.get(fiend).expect("standing").pos,
        stood,
        "a raze takes no aim, so there is nothing to walk into"
    );
    assert_eq!(
        hp(&world, out),
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
        Team::Dire,
        Vec2::from_ints(5000 + rules::PRESENCE_RADIUS + 500, 5000),
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
fn souls_outlive_the_death_of_the_one_who_gathered_them_but_for_a_share() {
    for (held, kept) in [(rules::SOULS_PER_UNIT, rules::SOULS_PER_UNIT), (10, 7)] {
        let (mut world, fiend, _mark) = fiend_and_a_mark(400);
        hand_souls(&mut world, fiend, held);
        world.bury(vec![(fiend, None)], &mut Vec::new());
        assert_eq!(world.seats[0].unit, None, "the body is gone");
        let back = back_on_its_feet(&mut world);
        assert_eq!(souls(&world, back), kept, "{held} held, {kept} come back");
    }
}

#[test]
fn a_requiem_lets_a_line_fly_for_every_soul_and_a_line_burns_what_it_crosses_once() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    hand_souls(&mut world, fiend, 5);
    world.step();
    let full = hp(&world, mark);
    let_go(&mut world, 5);
    let flying = |world: &World| {
        world
            .view_full()
            .projectiles
            .iter()
            .filter(|shown| shown.ability == Some(ability::REQUIEM))
            .count()
    };
    assert_eq!(flying(&world), 5, "one line to a soul");
    assert_eq!(souls(&world, fiend), 5, "and the souls are kept");
    // The line along the fiend's facing reaches the mark; the others fly
    // wide of it. His swings at it are not the requiem's and are left out.
    let mut burns = Vec::new();
    for _ in 0..60 {
        for event in world.step() {
            if let EventKind::Damaged {
                source,
                target,
                amount,
                kind: DamageKind::Magical,
                ..
            } = event.kind
                && source == Some(wire_id(fiend))
                && target == wire_id(mark)
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
    assert!(hp(&world, mark) < full, "and it was felt");
    assert_eq!(flying(&world), 0, "and the lines have flown out");
}

#[test]
fn what_stands_at_the_fiend_is_crossed_by_every_line_and_held_the_longest() {
    let (mut world, fiend, mark) = fiend_and_a_mark(60);
    hand_souls(&mut world, fiend, 5);
    world.step();
    let full = hp(&world, mark);
    let_go(&mut world, 5);
    world.step();
    let taken = full - hp(&world, mark);
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
        carries(&world, mark, ModifierKind::Slowed { pct: 0 }),
        "and it walks slower"
    );
    let projected = world
        .view(Team::Dire)
        .units
        .into_iter()
        .find(|unit| unit.id == wire_id(mark))
        .expect("the frightened creep is projected");
    assert_ne!(
        projected.statuses.bits & StatusFlags::FEARED,
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
        isqrt64(
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
                Some(ActionState::Attack { .. })
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
    let dire = world.view(Team::Dire);
    let landed = dire
        .projectiles
        .iter()
        .find(|shown| shown.ability == Some(ability::RAZE_FAR))
        .expect("the Dire side sees the raze where it landed");
    assert_eq!(
        landed.pos,
        Vec2::from_ints(5000 + rules::RAZE_DISTANCE[2], 5000)
    );
    assert!(
        dire.units.iter().all(|shown| shown.hero != Some(HeroId(2))),
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
            .all(|shown| shown.ability != Some(ability::RAZE_FAR)),
        "the mark is gone once its ticks have run"
    );
}

#[test]
fn a_requiem_with_no_souls_gathered_touches_nobody() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let full = hp(&world, mark);
    let_go(&mut world, 5);
    world.step();
    world.step();
    assert_eq!(hp(&world, mark), full, "nothing gathered is nothing let go");
    assert!(
        !carries(&world, mark, ModifierKind::Slowed { pct: 0 }),
        "not even the slow"
    );
    let spent = world
        .abilities
        .get(fiend)
        .map_or(0, |book| book.slots[5].cooldown);
    assert!(spent > 0, "though the cast itself happened");
}

#[test]
fn the_view_carries_what_has_been_gathered_as_a_counted_effect() {
    let (mut world, fiend, mark) = fiend_and_a_mark(400);
    let mut events = Vec::new();
    world.bury(vec![(mark, Some(fiend))], &mut events);
    world.step();
    let view = world.view(Team::Radiant);
    let shown = view
        .units
        .iter()
        .find(|u| u.hero == Some(HeroId(2)))
        .expect("he is in the view");
    let souls = shown
        .effects
        .iter()
        .find(|e| e.stacks.is_some())
        .expect("what is gathered is on him");
    assert_eq!(souls.stacks, Some(rules::SOULS_PER_UNIT));
    assert_eq!(souls.ticks_left, None, "and nothing counts it down");
}

/// What a unit has left of its health.
fn hp(world: &World, unit: Entity) -> Fixed {
    world.health.get(unit).expect("standing").hp
}

/// Souls a hero holds.
fn souls(world: &World, hero: Entity) -> u32 {
    world
        .stacks
        .get(hero)
        .map_or(0, |kept| kept.of(StackKind::Souls))
}
